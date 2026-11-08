//! In-memory stand-ins for the notification ports

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::error::Result;
use crate::models::{
    DevicePlatform, DeviceRegistration, Document, DocumentOutcome, Id, NotificationKind,
    NotificationPreferences, PendingNotification, PushDevice, PushDeviceState, PushEnvironment,
    PushToken, PushTokenKind, UserId, VaultId, VaultRole,
};
use crate::ports::{
    AccessPolicy, DocumentLookup, GrantLiveness, PushDeviceRepository, PushMessage, PushOutcome,
    PushTransport,
};

#[derive(Default)]
struct Store {
    devices: HashMap<Uuid, PushDevice>,
    outcomes: HashSet<(Id, &'static str)>,
    pending: Vec<PendingNotification>,
    states: HashMap<Uuid, PushDeviceState>,
    leases: HashMap<Uuid, DateTime<Utc>>,
}

#[derive(Default)]
pub struct MemoryDevices(Mutex<Store>);

impl MemoryDevices {
    pub fn add(&self, device: PushDevice) {
        self.0.lock().unwrap().devices.insert(device.id, device);
    }

    pub fn all_pending(&self) -> Vec<PendingNotification> {
        self.0.lock().unwrap().pending.clone()
    }

    pub fn device(&self, id: Uuid) -> Option<PushDevice> {
        self.0.lock().unwrap().devices.get(&id).cloned()
    }

    pub fn set_preferences(&self, id: Uuid, preferences: NotificationPreferences) {
        if let Some(device) = self.0.lock().unwrap().devices.get_mut(&id) {
            device.preferences = preferences;
        }
    }
}

fn remove_device(store: &mut Store, id: Uuid) {
    store.devices.remove(&id);
    store.pending.retain(|p| p.device_id != id);
    store.states.remove(&id);
    store.leases.remove(&id);
}

#[async_trait]
impl PushDeviceRepository for MemoryDevices {
    async fn upsert(&self, r: DeviceRegistration) -> Result<(PushDevice, bool)> {
        let now = Utc::now();
        let mut store = self.0.lock().unwrap();
        let existing = store
            .devices
            .values()
            .find(|d| d.user_id == r.user_id && d.installation_id == r.installation_id)
            .cloned();
        let created = existing.is_none();
        let mut device = existing.unwrap_or(PushDevice {
            id: Uuid::new_v4(),
            user_id: r.user_id.clone(),
            installation_id: r.installation_id.clone(),
            client_id: String::new(),
            grant_id: String::new(),
            platform: r.platform,
            environment: r.environment,
            locale: String::new(),
            preferences: NotificationPreferences::default(),
            alert_token: None,
            widget_token: None,
            created_at: now,
            updated_at: now,
            last_delivered_at: None,
        });
        device.client_id = r.client_id;
        device.grant_id = r.grant_id;
        device.platform = r.platform;
        device.environment = r.environment;
        device.locale = r.locale;
        device.preferences = r.preferences;
        device.updated_at = now;
        if let Some(value) = r.alert_token {
            device.alert_token = Some(PushToken {
                value,
                registered_at: now,
            });
        }
        if let Some(value) = r.widget_token {
            device.widget_token = Some(PushToken {
                value,
                registered_at: now,
            });
        }
        store.devices.insert(device.id, device.clone());
        Ok((device, created))
    }

    async fn get(&self, id: Uuid) -> Result<Option<PushDevice>> {
        Ok(self.device(id))
    }

    async fn find(&self, user: &UserId, installation_id: &str) -> Result<Option<PushDevice>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .devices
            .values()
            .find(|d| &d.user_id == user && d.installation_id == installation_id)
            .cloned())
    }

    async fn list_for_user(&self, user: &UserId) -> Result<Vec<PushDevice>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .devices
            .values()
            .filter(|d| &d.user_id == user)
            .cloned()
            .collect())
    }

    async fn list_all(&self) -> Result<Vec<PushDevice>> {
        Ok(self.0.lock().unwrap().devices.values().cloned().collect())
    }

    async fn delete(&self, user: &UserId, installation_id: &str) -> Result<bool> {
        let found = self.find(user, installation_id).await?;
        if let Some(device) = &found {
            remove_device(&mut self.0.lock().unwrap(), device.id);
        }
        Ok(found.is_some())
    }

    async fn delete_by_id(&self, id: Uuid) -> Result<()> {
        remove_device(&mut self.0.lock().unwrap(), id);
        Ok(())
    }

    async fn clear_token(
        &self,
        id: Uuid,
        kind: PushTokenKind,
        inactive_since: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        let mut store = self.0.lock().unwrap();
        let Some(device) = store.devices.get_mut(&id) else {
            return Ok(false);
        };
        let slot = match kind {
            PushTokenKind::Alert => &mut device.alert_token,
            PushTokenKind::Widget => &mut device.widget_token,
        };
        let clear = slot
            .as_ref()
            .is_some_and(|t| inactive_since.is_none_or(|since| since > t.registered_at));
        if clear {
            *slot = None;
        }
        if device.alert_token.is_none() && device.widget_token.is_none() {
            remove_device(&mut store, id);
        }
        Ok(clear)
    }

    async fn record_first_outcome(
        &self,
        document: Id,
        outcome: DocumentOutcome,
        pending: &[PendingNotification],
    ) -> Result<bool> {
        let mut store = self.0.lock().unwrap();
        if !store.outcomes.insert((document, outcome.as_str())) {
            return Ok(false);
        }
        for item in pending {
            let duplicate = store.pending.iter().any(|p| {
                p.device_id == item.device_id
                    && p.document_id == item.document_id
                    && p.kind == item.kind
            });
            if !duplicate {
                store.pending.push(item.clone());
            }
        }
        Ok(true)
    }

    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        quiet: Duration,
        ceiling: Duration,
        lease: Duration,
        limit: u32,
    ) -> Result<Vec<Uuid>> {
        let mut store = self.0.lock().unwrap();
        let mut ids: Vec<Uuid> = store.pending.iter().map(|p| p.device_id).collect();
        ids.sort();
        ids.dedup();
        let mut claimed = Vec::new();
        for id in ids {
            let times: Vec<_> = store
                .pending
                .iter()
                .filter(|p| p.device_id == id)
                .map(|p| p.created_at)
                .collect();
            let newest = *times.iter().max().unwrap();
            let oldest = *times.iter().min().unwrap();
            let due = newest <= now - quiet || oldest <= now - ceiling;
            let leased = store.leases.get(&id).is_some_and(|until| *until > now);
            let backing_off = store
                .states
                .get(&id)
                .and_then(|s| s.backoff_until)
                .is_some_and(|until| until > now);
            if due && !leased && !backing_off && claimed.len() < limit as usize {
                store.leases.insert(id, now + lease);
                claimed.push(id);
            }
        }
        Ok(claimed)
    }

    async fn claim_widget_due(
        &self,
        now: DateTime<Utc>,
        min_interval: Duration,
        lease: Duration,
        limit: u32,
    ) -> Result<Vec<Uuid>> {
        let mut store = self.0.lock().unwrap();
        let mut claimed = Vec::new();
        let ids: Vec<Uuid> = store.devices.keys().copied().collect();
        for id in ids {
            let state = store.states.get(&id).cloned().unwrap_or_default();
            let has_token = store.devices[&id].widget_token.is_some();
            let due = state.widgets_dirty_at.is_some()
                && state
                    .widgets_sent_at
                    .is_none_or(|sent| sent <= now - min_interval);
            let leased = store.leases.get(&id).is_some_and(|until| *until > now);
            if has_token && due && !leased && claimed.len() < limit as usize {
                store.leases.insert(id, now + lease);
                claimed.push(id);
            }
        }
        Ok(claimed)
    }

    async fn pending(&self, device: Uuid) -> Result<Vec<PendingNotification>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .pending
            .iter()
            .filter(|p| p.device_id == device)
            .cloned()
            .collect())
    }

    async fn remove_pending(&self, device: Uuid, items: &[(Id, NotificationKind)]) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .pending
            .retain(|p| p.device_id != device || !items.contains(&(p.document_id, p.kind)));
        Ok(())
    }

    async fn state(&self, device: Uuid) -> Result<PushDeviceState> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .states
            .get(&device)
            .cloned()
            .unwrap_or_default())
    }

    async fn release(&self, device: Uuid, state: &PushDeviceState) -> Result<()> {
        let mut store = self.0.lock().unwrap();
        let previous = store.states.get(&device).cloned().unwrap_or_default();
        let mut state = state.clone();
        // Widgets dirtied while the lease was held stay dirty.
        if previous.widgets_dirty_at > state.widgets_sent_at {
            state.widgets_dirty_at = state.widgets_dirty_at.or(previous.widgets_dirty_at);
        }
        store.states.insert(device, state);
        store.leases.remove(&device);
        Ok(())
    }

    async fn mark_delivered(&self, device: Uuid, at: DateTime<Utc>) -> Result<()> {
        if let Some(device) = self.0.lock().unwrap().devices.get_mut(&device) {
            device.last_delivered_at = Some(at);
        }
        Ok(())
    }

    async fn mark_widgets_dirty(&self, devices: &[Uuid], at: DateTime<Utc>) -> Result<()> {
        let mut store = self.0.lock().unwrap();
        for id in devices {
            store.states.entry(*id).or_default().widgets_dirty_at = Some(at);
        }
        Ok(())
    }

    async fn try_record_test(
        &self,
        device: Uuid,
        at: DateTime<Utc>,
        not_since: DateTime<Utc>,
    ) -> Result<bool> {
        let mut store = self.0.lock().unwrap();
        let state = store.states.entry(device).or_default();
        if state.test_sent_at.is_some_and(|sent| sent > not_since) {
            return Ok(false);
        }
        state.test_sent_at = Some(at);
        Ok(true)
    }

    async fn prune_pending(&self, before: DateTime<Utc>) -> Result<u64> {
        let mut store = self.0.lock().unwrap();
        let count = store.pending.len();
        store.pending.retain(|p| p.created_at >= before);
        Ok((count - store.pending.len()) as u64)
    }

    async fn count_devices(&self) -> Result<u64> {
        Ok(self.0.lock().unwrap().devices.len() as u64)
    }
}

#[derive(Default)]
pub struct MemoryDocuments(pub Mutex<HashMap<Id, Document>>);

impl MemoryDocuments {
    pub fn put(&self, document: Document) {
        self.0.lock().unwrap().insert(document.id, document);
    }

    pub fn update(&self, id: Id, change: impl FnOnce(&mut Document)) {
        if let Some(document) = self.0.lock().unwrap().get_mut(&id) {
            change(document);
        }
    }
}

#[async_trait]
impl DocumentLookup for MemoryDocuments {
    async fn document(&self, vault: VaultId, id: Id) -> Result<Option<Document>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&id)
            .filter(|d| d.vault_id == vault && d.deleted_at.is_none())
            .cloned())
    }
}

#[derive(Default)]
pub struct Roles(pub Mutex<HashMap<(UserId, VaultId), VaultRole>>);

impl Roles {
    pub fn grant(&self, user: &UserId, vault: VaultId, role: VaultRole) {
        self.0.lock().unwrap().insert((user.clone(), vault), role);
    }

    pub fn revoke(&self, user: &UserId, vault: VaultId) {
        self.0.lock().unwrap().remove(&(user.clone(), vault));
    }
}

#[async_trait]
impl AccessPolicy for Roles {
    async fn effective_role(&self, user: &UserId, vault: VaultId) -> Result<Option<VaultRole>> {
        Ok(self.0.lock().unwrap().get(&(user.clone(), vault)).copied())
    }
}

#[derive(Default)]
pub struct Grants(pub Mutex<HashSet<String>>);

impl Grants {
    pub fn end(&self, grant: &str) {
        self.0.lock().unwrap().insert(grant.to_string());
    }
}

#[async_trait]
impl GrantLiveness for Grants {
    async fn is_live(&self, grant_id: &str) -> Result<bool> {
        Ok(!self.0.lock().unwrap().contains(grant_id))
    }
}

/// Records what it sends and answers with scripted outcomes, `Delivered` by default
pub struct Recorder {
    pub sent: Mutex<Vec<PushMessage>>,
    pub outcomes: Mutex<VecDeque<PushOutcome>>,
    pub available: Mutex<bool>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            sent: Mutex::default(),
            outcomes: Mutex::default(),
            available: Mutex::new(true),
        }
    }
}

impl Recorder {
    pub fn sent(&self) -> Vec<PushMessage> {
        self.sent.lock().unwrap().clone()
    }

    pub fn answer(&self, outcome: PushOutcome) {
        self.outcomes.lock().unwrap().push_back(outcome);
    }
}

#[async_trait]
impl PushTransport for Recorder {
    async fn available(&self) -> bool {
        *self.available.lock().unwrap()
    }

    async fn send(&self, message: &PushMessage) -> PushOutcome {
        self.sent.lock().unwrap().push(message.clone());
        self.outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(PushOutcome::Delivered)
    }
}

pub fn device(user: &str, created_at: DateTime<Utc>) -> PushDevice {
    PushDevice {
        id: Uuid::new_v4(),
        user_id: UserId::from(user),
        installation_id: Uuid::new_v4().to_string(),
        client_id: "client".into(),
        grant_id: format!("grant-{user}"),
        platform: DevicePlatform::Ios,
        environment: PushEnvironment::Production,
        locale: "en".into(),
        preferences: NotificationPreferences::default(),
        alert_token: Some(PushToken {
            value: "ab".repeat(32),
            registered_at: created_at,
        }),
        widget_token: None,
        created_at,
        updated_at: created_at,
        last_delivered_at: None,
    }
}

pub fn document(vault: VaultId, uploaded_by: Option<&str>, created_at: DateTime<Utc>) -> Document {
    let mut document = Document::new(vault, format!("Secret title {}", Uuid::new_v4()));
    document.uploaded_by = uploaded_by.map(UserId::from);
    document.created_at = created_at;
    document
}
