//! Sends what the planner queued: one alert per device per flush

use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration, Utc};
use tokio::sync::{watch, Notify};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::planner::{may_edit, EditCache};
use super::templates::{test_payload, widgets_payload, AlertSummary};
use crate::error::Result;
use crate::models::{Id, NotificationKind, NotificationScope, PushDevice, PushTokenKind, UserId};
use crate::ports::{
    AccessPolicy, DocumentLookup, GrantLiveness, PushDeviceRepository, PushLimitsSource,
    PushMessage, PushOutcome, PushTransport, PushType,
};

/// Fixed timing of the sender; the quiet window and hourly cap come from settings
#[derive(Debug, Clone)]
pub struct SenderConfig {
    /// A batch is sent at the latest this long after its first notification
    pub ceiling: Duration,
    pub lease: Duration,
    pub widget_interval: Duration,
    /// Pending notifications older than this are dropped
    pub max_age: Duration,
    /// Pushes stay deliverable this long
    pub expiration: Duration,
    pub tick: StdDuration,
    pub batch: u32,
}

impl Default for SenderConfig {
    fn default() -> Self {
        Self {
            ceiling: Duration::minutes(5),
            lease: Duration::seconds(60),
            widget_interval: Duration::minutes(15),
            max_age: Duration::hours(24),
            expiration: Duration::hours(24),
            tick: StdDuration::from_secs(5),
            batch: 50,
        }
    }
}

/// Delay after the `failures`-th failed attempt: 15 s doubling up to 15 minutes
fn backoff(failures: u32) -> Duration {
    Duration::seconds((15_i64 << failures.saturating_sub(1).min(6)).min(900))
}

/// Why a test notification was not sent
#[derive(Debug, PartialEq, Eq)]
pub enum TestRefusal {
    NotFound,
    Unavailable,
    TooSoon,
    NoToken,
}

/// One flush: the sender's claim on its devices and when it started
struct Pass {
    holder: Uuid,
    now: DateTime<Utc>,
    started: Instant,
}

/// What sending to one device came to
enum Flushed {
    Sent(usize),
    /// APNs refused the provider credentials, so nothing more goes out this round
    Refused,
}

pub struct NotificationSender {
    devices: Arc<dyn PushDeviceRepository>,
    documents: Arc<dyn DocumentLookup>,
    access: Arc<dyn AccessPolicy>,
    grants: Arc<dyn GrantLiveness>,
    transport: Arc<dyn PushTransport>,
    limits: Arc<dyn PushLimitsSource>,
    config: SenderConfig,
}

impl NotificationSender {
    pub fn new(
        devices: Arc<dyn PushDeviceRepository>,
        documents: Arc<dyn DocumentLookup>,
        access: Arc<dyn AccessPolicy>,
        grants: Arc<dyn GrantLiveness>,
        transport: Arc<dyn PushTransport>,
        limits: Arc<dyn PushLimitsSource>,
        config: SenderConfig,
    ) -> Self {
        Self {
            devices,
            documents,
            access,
            grants,
            transport,
            limits,
            config,
        }
    }

    /// Send what is due at `now`; returns the number of pushes sent
    pub async fn flush(&self, now: DateTime<Utc>) -> Result<usize> {
        if !self.transport.available().await {
            return Ok(0);
        }
        let limits = self.limits.limits().await;
        let pass = Pass {
            holder: Uuid::new_v4(),
            now,
            started: Instant::now(),
        };
        let mut sent = 0;
        let mut refused = false;
        for device in self
            .devices
            .claim_due(
                now,
                limits.quiet_window,
                self.config.ceiling,
                self.config.lease,
                self.config.batch,
                pass.holder,
            )
            .await?
        {
            if refused {
                self.unclaim(device, &pass).await?;
                continue;
            }
            match self
                .flush_device(device, &pass, limits.max_alerts_per_hour)
                .await?
            {
                Flushed::Sent(count) => sent += count,
                Flushed::Refused => refused = true,
            }
        }
        if refused {
            return Ok(sent);
        }
        for device in self
            .devices
            .claim_widget_due(
                now,
                self.config.widget_interval,
                self.config.lease,
                self.config.batch,
                pass.holder,
            )
            .await?
        {
            if refused {
                self.unclaim(device, &pass).await?;
                continue;
            }
            match self.flush_widgets(device, &pass).await? {
                Flushed::Sent(count) => sent += count,
                Flushed::Refused => refused = true,
            }
        }
        Ok(sent)
    }

    /// Give back a claimed device untouched, for the next flush
    async fn unclaim(&self, id: Uuid, pass: &Pass) -> Result<()> {
        let state = self.devices.state(id).await?;
        self.devices.release(id, pass.holder, &state).await?;
        Ok(())
    }

    /// Extend the lease before a send; `false` when another sender has taken the device
    async fn renew(&self, id: Uuid, pass: &Pass) -> Result<bool> {
        let elapsed = Duration::from_std(pass.started.elapsed()).unwrap_or(Duration::zero());
        let until = pass.now + elapsed + self.config.lease;
        let renewed = self.devices.renew(id, pass.holder, until).await?;
        if !renewed {
            debug!(device = %id, "Another sender took over the device");
        }
        Ok(renewed)
    }

    /// The device if its grant still authorizes the app; otherwise it is deleted
    async fn live_device(&self, id: Uuid) -> Result<Option<PushDevice>> {
        let Some(device) = self.devices.get(id).await? else {
            return Ok(None);
        };
        if !self.grants.is_live(&device.grant_id).await? {
            info!(device = %id, "Deleting a device whose authorization ended");
            self.devices.delete_by_id(id).await?;
            return Ok(None);
        }
        Ok(Some(device))
    }

    async fn flush_device(&self, id: Uuid, pass: &Pass, cap: u32) -> Result<Flushed> {
        let now = pass.now;
        let Some(device) = self.live_device(id).await? else {
            return Ok(Flushed::Sent(0));
        };
        let mut state = self.devices.state(id).await?;
        let (summary, stale) = self.summarize(&device).await?;
        if !stale.is_empty() {
            self.devices.remove_pending(id, &stale).await?;
        }
        let Some(token) = device.alert_token.as_ref().filter(|_| !summary.is_empty()) else {
            self.devices.remove_pending(id, &summary.items()).await?;
            state.batch_id = None;
            self.devices.release(id, pass.holder, &state).await?;
            return Ok(Flushed::Sent(0));
        };

        let window_open = state
            .window_started_at
            .is_some_and(|start| now < start + Duration::hours(1));
        if !window_open {
            state.window_started_at = Some(now);
            state.alerts_in_window = 0;
        }
        if state.alerts_in_window >= cap {
            // Held notifications roll into the first alert the window allows.
            state.backoff_until = state
                .window_started_at
                .map(|start| start + Duration::hours(1));
            self.devices.release(id, pass.holder, &state).await?;
            return Ok(Flushed::Sent(0));
        }

        if !self.renew(id, pass).await? {
            return Ok(Flushed::Sent(0));
        }
        // A retry reuses the collapse id, so an attempt that did reach APNs is replaced.
        let batch = *state.batch_id.get_or_insert_with(Uuid::new_v4);
        let message = PushMessage {
            apns_id: Uuid::new_v4(),
            token: token.value.clone(),
            environment: device.environment,
            push_type: PushType::Alert,
            priority: summary.priority(),
            collapse_id: Some(batch.to_string()),
            expiration: now + self.config.expiration,
            payload: summary.payload(&device.locale, device.preferences.show_titles),
        };
        let outcome = self.transport.send(&message).await;
        debug!(device = %id, apns_id = %message.apns_id, ?outcome, "Sent alert");
        let items = summary.items();
        let sent = match outcome {
            PushOutcome::Delivered => {
                self.devices.remove_pending(id, &items).await?;
                self.devices.mark_delivered(id, now).await?;
                state.alerts_in_window += 1;
                state.failures = 0;
                state.backoff_until = None;
                state.batch_id = None;
                1
            }
            PushOutcome::TokenInvalid { since } => {
                self.devices.remove_pending(id, &items).await?;
                self.devices
                    .clear_token(id, PushTokenKind::Alert, since)
                    .await?;
                state.batch_id = None;
                0
            }
            PushOutcome::Rejected { reason } => {
                warn!(device = %id, apns_id = %message.apns_id, %reason, "APNs rejected an alert");
                self.devices.remove_pending(id, &items).await?;
                state.batch_id = None;
                0
            }
            PushOutcome::Retry { reason } => {
                state.failures += 1;
                state.backoff_until = Some(now + backoff(state.failures));
                debug!(device = %id, %reason, "Retrying an alert later");
                0
            }
            PushOutcome::CredentialError { reason } => {
                warn!(%reason, "APNs refused the provider credentials");
                self.devices.release(id, pass.holder, &state).await?;
                return Ok(Flushed::Refused);
            }
        };
        if self.devices.get(id).await?.is_some() {
            self.devices.release(id, pass.holder, &state).await?;
        }
        Ok(Flushed::Sent(sent))
    }

    /// What is still worth telling, and the pending rows that are not
    async fn summarize(
        &self,
        device: &PushDevice,
    ) -> Result<(AlertSummary, Vec<(Id, NotificationKind)>)> {
        let mut summary = AlertSummary::default();
        let mut stale = Vec::new();
        let mut editors = EditCache::new();
        let preferences = &device.preferences;
        for item in self.devices.pending(device.id).await? {
            let document = self
                .documents
                .document(item.vault_id, item.document_id)
                .await?;
            let keep = match &document {
                None => false,
                Some(document) => {
                    preferences.wants(item.kind)
                        && preferences.vaults.includes(document.vault_id)
                        && !(item.kind == NotificationKind::NeedsReview
                            && document.reviewed_at.is_some())
                        && (preferences.scope == NotificationScope::Editable
                            || document.uploaded_by.as_ref() == Some(&device.user_id))
                        && may_edit(&*self.access, &device.user_id, document, &mut editors).await?
                }
            };
            if keep {
                summary.add(item.kind, item.document_id, item.vault_id);
            } else {
                stale.push((item.document_id, item.kind));
            }
        }
        Ok((summary, stale))
    }

    async fn flush_widgets(&self, id: Uuid, pass: &Pass) -> Result<Flushed> {
        let now = pass.now;
        let Some(device) = self.live_device(id).await? else {
            return Ok(Flushed::Sent(0));
        };
        let mut state = self.devices.state(id).await?;
        let Some(token) = device.widget_token.as_ref() else {
            state.widgets_dirty_at = None;
            self.devices.release(id, pass.holder, &state).await?;
            return Ok(Flushed::Sent(0));
        };
        if !self.renew(id, pass).await? {
            return Ok(Flushed::Sent(0));
        }
        let message = PushMessage {
            apns_id: Uuid::new_v4(),
            token: token.value.clone(),
            environment: device.environment,
            push_type: PushType::Widgets,
            priority: 5,
            collapse_id: None,
            expiration: now + self.config.widget_interval,
            payload: widgets_payload(),
        };
        let outcome = self.transport.send(&message).await;
        let sent = match outcome {
            PushOutcome::Delivered => {
                state.widgets_dirty_at = None;
                state.widgets_sent_at = Some(now);
                1
            }
            PushOutcome::TokenInvalid { since } => {
                state.widgets_dirty_at = None;
                self.devices
                    .clear_token(id, PushTokenKind::Widget, since)
                    .await?;
                0
            }
            PushOutcome::Rejected { reason } => {
                warn!(device = %id, %reason, "APNs rejected a widget reload");
                state.widgets_dirty_at = None;
                0
            }
            // Try again at the next interval.
            PushOutcome::Retry { .. } => {
                state.widgets_sent_at = Some(now);
                0
            }
            PushOutcome::CredentialError { reason } => {
                warn!(%reason, "APNs refused the provider credentials");
                self.devices.release(id, pass.holder, &state).await?;
                return Ok(Flushed::Refused);
            }
        };
        if self.devices.get(id).await?.is_some() {
            self.devices.release(id, pass.holder, &state).await?;
        }
        Ok(Flushed::Sent(sent))
    }

    /// Send a test alert to a person's device right away
    pub async fn send_test(
        &self,
        user: &UserId,
        installation_id: &str,
        now: DateTime<Utc>,
    ) -> std::result::Result<(), TestRefusal> {
        let device = self
            .devices
            .find(user, installation_id)
            .await
            .ok()
            .flatten()
            .ok_or(TestRefusal::NotFound)?;
        if !self.transport.available().await {
            return Err(TestRefusal::Unavailable);
        }
        let token = device.alert_token.clone().ok_or(TestRefusal::NoToken)?;
        let recorded = self
            .devices
            .try_record_test(device.id, now, now - Duration::minutes(1))
            .await
            .map_err(|_| TestRefusal::Unavailable)?;
        if !recorded {
            return Err(TestRefusal::TooSoon);
        }
        let message = PushMessage {
            apns_id: Uuid::new_v4(),
            token: token.value,
            environment: device.environment,
            push_type: PushType::Alert,
            priority: 10,
            collapse_id: None,
            expiration: now + Duration::minutes(10),
            payload: test_payload(&device.locale),
        };
        let transport = self.transport.clone();
        let devices = self.devices.clone();
        tokio::spawn(async move {
            if let PushOutcome::TokenInvalid { since } = transport.send(&message).await {
                let _ = devices
                    .clear_token(device.id, PushTokenKind::Alert, since)
                    .await;
            }
        });
        Ok(())
    }

    pub async fn available(&self) -> bool {
        self.transport.available().await
    }

    /// Flush whenever woken or every tick, prune hourly, until shutdown
    pub async fn run(self: Arc<Self>, wake: Arc<Notify>, mut shutdown: watch::Receiver<bool>) {
        let mut last_prune = Utc::now() - Duration::hours(1);
        loop {
            let now = Utc::now();
            if let Err(e) = self.flush(now).await {
                warn!("Push notification flush failed: {e}");
            }
            if now - last_prune >= Duration::hours(1) {
                last_prune = now;
                match self.devices.prune_pending(now - self.config.max_age).await {
                    Ok(0) => {}
                    Ok(n) => debug!("Dropped {n} expired pending notifications"),
                    Err(e) => warn!("Pruning pending notifications failed: {e}"),
                }
            }
            tokio::select! {
                _ = shutdown.changed() => return,
                _ = wake.notified() => {}
                _ = tokio::time::sleep(self.config.tick) => {}
            }
            if *shutdown.borrow() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests;
