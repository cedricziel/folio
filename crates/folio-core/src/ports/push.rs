//! Push notification storage and delivery

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::error::Result;
use crate::models::{
    DeviceRegistration, Document, DocumentOutcome, Id, NotificationKind, PendingNotification,
    PushDevice, PushDeviceState, PushEnvironment, PushTokenKind, UserId, VaultId,
};

/// Devices, the first outcomes of documents and what is pending for each device
#[async_trait]
pub trait PushDeviceRepository: Send + Sync {
    /// Insert or update the device of `(user, installation)`; `true` when it was created
    ///
    /// Only the tokens present in the registration change, each with its own
    /// registration time. A token held by another row of the server is taken
    /// from it, and that row is deleted when it has no token left.
    async fn upsert(&self, registration: DeviceRegistration) -> Result<(PushDevice, bool)>;

    async fn get(&self, id: Uuid) -> Result<Option<PushDevice>>;

    async fn find(&self, user: &UserId, installation_id: &str) -> Result<Option<PushDevice>>;

    async fn list_for_user(&self, user: &UserId) -> Result<Vec<PushDevice>>;

    /// Every device, for working out who hears about a document
    async fn list_all(&self) -> Result<Vec<PushDevice>>;

    /// Delete a person's device and what is pending for it; `false` when there was none
    async fn delete(&self, user: &UserId, installation_id: &str) -> Result<bool>;

    async fn delete_by_id(&self, id: Uuid) -> Result<()>;

    /// Forget a token APNs called inactive at `inactive_since`
    ///
    /// A token registered after that time was re-registered and is kept. A
    /// device with no token left is deleted. Returns whether the token was cleared.
    async fn clear_token(
        &self,
        device: Uuid,
        kind: PushTokenKind,
        inactive_since: Option<DateTime<Utc>>,
    ) -> Result<bool>;

    /// Record the first `outcome` of a document and queue `pending` with it
    ///
    /// Returns `false`, and queues nothing, when the outcome was already
    /// recorded. Pending rows are unique per device, document and kind.
    async fn record_first_outcome(
        &self,
        document: Id,
        outcome: DocumentOutcome,
        pending: &[PendingNotification],
    ) -> Result<bool>;

    /// Claim devices whose pending notifications are due, for `lease`
    ///
    /// A device is due when its newest pending row is older than `quiet`, or
    /// its oldest older than `ceiling`, and it is neither leased nor backing off.
    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        quiet: Duration,
        ceiling: Duration,
        lease: Duration,
        limit: u32,
    ) -> Result<Vec<Uuid>>;

    /// Claim devices with a widget token whose widgets are waiting for a reload
    async fn claim_widget_due(
        &self,
        now: DateTime<Utc>,
        min_interval: Duration,
        lease: Duration,
        limit: u32,
    ) -> Result<Vec<Uuid>>;

    async fn pending(&self, device: Uuid) -> Result<Vec<PendingNotification>>;

    async fn remove_pending(&self, device: Uuid, items: &[(Id, NotificationKind)]) -> Result<()>;

    async fn state(&self, device: Uuid) -> Result<PushDeviceState>;

    /// Store the state and end the lease
    async fn release(&self, device: Uuid, state: &PushDeviceState) -> Result<()>;

    async fn mark_delivered(&self, device: Uuid, at: DateTime<Utc>) -> Result<()>;

    /// Flag the widgets of these devices for a reload
    async fn mark_widgets_dirty(&self, devices: &[Uuid], at: DateTime<Utc>) -> Result<()>;

    /// Record a test notification unless one was sent after `not_since`; `true` when recorded
    async fn try_record_test(
        &self,
        device: Uuid,
        at: DateTime<Utc>,
        not_since: DateTime<Utc>,
    ) -> Result<bool>;

    /// Drop pending notifications created before `before`
    async fn prune_pending(&self, before: DateTime<Utc>) -> Result<u64>;

    async fn count_devices(&self) -> Result<u64>;
}

/// Reads the current state of a document, as notifications need it
#[async_trait]
pub trait DocumentLookup: Send + Sync {
    /// The document if it is in `vault` and not trashed
    async fn document(&self, vault: VaultId, id: Id) -> Result<Option<Document>>;
}

/// [`DocumentLookup`] over the document repository
pub struct RepositoryDocuments(pub std::sync::Arc<dyn crate::ports::DocumentRepository>);

#[async_trait]
impl DocumentLookup for RepositoryDocuments {
    async fn document(&self, vault: VaultId, id: Id) -> Result<Option<Document>> {
        self.0.find_by_id(vault, id).await
    }
}

/// Whether an OAuth grant still authorizes its app
#[async_trait]
pub trait GrantLiveness: Send + Sync {
    /// `true` while the grant has an unexpired access token or an unused, unexpired refresh token
    async fn is_live(&self, grant_id: &str) -> Result<bool>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushType {
    Alert,
    Widgets,
}

/// One push to one token
#[derive(Debug, Clone, PartialEq)]
pub struct PushMessage {
    pub apns_id: Uuid,
    pub token: String,
    pub environment: PushEnvironment,
    pub push_type: PushType,
    pub priority: u8,
    pub collapse_id: Option<String>,
    pub expiration: DateTime<Utc>,
    pub payload: serde_json::Value,
}

/// What became of a push
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    Delivered,
    /// The token is no longer valid; `since` is when APNs says it stopped being valid
    TokenInvalid {
        since: Option<DateTime<Utc>>,
    },
    /// Try again later
    Retry {
        reason: String,
    },
    /// The provider credentials were refused; nothing can be sent until they change
    CredentialError {
        reason: String,
    },
    /// APNs refused this push for good
    Rejected {
        reason: String,
    },
}

/// Delivers pushes to APNs, or nowhere
#[async_trait]
pub trait PushTransport: Send + Sync {
    /// Whether pushes can be sent right now
    async fn available(&self) -> bool;

    async fn send(&self, message: &PushMessage) -> PushOutcome;
}

/// Coalescing and rate limits, which settings may change at runtime
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushLimits {
    pub quiet_window: Duration,
    pub max_alerts_per_hour: u32,
}

impl Default for PushLimits {
    fn default() -> Self {
        Self {
            quiet_window: Duration::seconds(30),
            max_alerts_per_hour: 12,
        }
    }
}

#[async_trait]
pub trait PushLimitsSource: Send + Sync {
    async fn limits(&self) -> PushLimits;
}

#[async_trait]
impl PushLimitsSource for PushLimits {
    async fn limits(&self) -> PushLimits {
        *self
    }
}
