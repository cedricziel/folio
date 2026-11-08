//! Devices registered for push notifications and what is pending for them

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Id, UserId, VaultId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DevicePlatform {
    Ios,
    Macos,
}

impl DevicePlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Macos => "macos",
        }
    }
}

impl std::str::FromStr for DevicePlatform {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ios" => Ok(Self::Ios),
            "macos" => Ok(Self::Macos),
            other => Err(format!("unknown platform '{other}'")),
        }
    }
}

/// The APNs environment a token was issued for
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PushEnvironment {
    Sandbox,
    Production,
}

impl PushEnvironment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Production => "production",
        }
    }
}

impl std::str::FromStr for PushEnvironment {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sandbox" => Ok(Self::Sandbox),
            "production" => Ok(Self::Production),
            other => Err(format!("unknown environment '{other}'")),
        }
    }
}

/// Whose documents a device hears about
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum NotificationScope {
    /// Documents the person uploaded
    #[default]
    Mine,
    /// Every document the person may edit
    Editable,
}

/// Which vaults a device hears about: `"all"` or a list of vault IDs
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum VaultSelection {
    #[default]
    All,
    Only(Vec<VaultId>),
}

impl VaultSelection {
    pub fn includes(&self, vault: VaultId) -> bool {
        match self {
            Self::All => true,
            Self::Only(vaults) => vaults.contains(&vault),
        }
    }
}

impl Serialize for VaultSelection {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::All => serializer.serialize_str("all"),
            Self::Only(vaults) => vaults.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for VaultSelection {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Word(String),
            List(Vec<VaultId>),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Word(word) if word == "all" => Ok(Self::All),
            Raw::Word(word) => Err(serde::de::Error::custom(format!(
                "expected \"all\" or a list of vault IDs, got \"{word}\""
            ))),
            Raw::List(vaults) => Ok(Self::Only(vaults)),
        }
    }
}

#[cfg(feature = "openapi")]
impl utoipa::PartialSchema for VaultSelection {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{ArrayBuilder, ObjectBuilder, OneOfBuilder, Type};
        OneOfBuilder::new()
            .item(
                ObjectBuilder::new()
                    .schema_type(Type::String)
                    .enum_values(Some(["all"])),
            )
            .item(
                ArrayBuilder::new().items(ObjectBuilder::new().schema_type(Type::String).format(
                    Some(utoipa::openapi::SchemaFormat::KnownFormat(
                        utoipa::openapi::KnownFormat::Uuid,
                    )),
                )),
            )
            .description(Some("\"all\" or a list of vault IDs"))
            .into()
    }
}

#[cfg(feature = "openapi")]
impl utoipa::ToSchema for VaultSelection {}

/// What a device wants to hear about
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(default)]
pub struct NotificationPreferences {
    /// First processing finished on a reviewed document
    pub processed: bool,
    /// First processing finished on an unreviewed document
    pub needs_review: bool,
    /// Processing failed after its retries
    pub failed: bool,
    pub scope: NotificationScope,
    pub vaults: VaultSelection,
    /// Let the device fetch and show document titles
    pub show_titles: bool,
}

impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            processed: false,
            needs_review: true,
            failed: true,
            scope: NotificationScope::Mine,
            vaults: VaultSelection::All,
            show_titles: false,
        }
    }
}

impl NotificationPreferences {
    pub fn wants(&self, kind: NotificationKind) -> bool {
        match kind {
            NotificationKind::Processed => self.processed,
            NotificationKind::NeedsReview => self.needs_review,
            NotificationKind::Failed => self.failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    NeedsReview,
    Failed,
    Processed,
}

impl NotificationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Processed => "processed",
            Self::NeedsReview => "needs_review",
            Self::Failed => "failed",
        }
    }
}

impl std::str::FromStr for NotificationKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "processed" => Ok(Self::Processed),
            "needs_review" => Ok(Self::NeedsReview),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown notification kind '{other}'")),
        }
    }
}

/// Which first outcome of a document was recorded
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentOutcome {
    Completed,
    Failed,
}

impl DocumentOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushTokenKind {
    Alert,
    Widget,
}

/// An APNs token and when it was last registered
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushToken {
    pub value: String,
    pub registered_at: DateTime<Utc>,
}

/// A registered installation of the app for one person
#[derive(Debug, Clone, PartialEq)]
pub struct PushDevice {
    pub id: Uuid,
    pub user_id: UserId,
    pub installation_id: String,
    pub client_id: String,
    pub grant_id: String,
    pub platform: DevicePlatform,
    pub environment: PushEnvironment,
    pub locale: String,
    pub preferences: NotificationPreferences,
    pub alert_token: Option<PushToken>,
    pub widget_token: Option<PushToken>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_delivered_at: Option<DateTime<Utc>>,
}

/// What a registration request asks to store
#[derive(Debug, Clone)]
pub struct DeviceRegistration {
    pub user_id: UserId,
    pub installation_id: String,
    pub client_id: String,
    pub grant_id: String,
    pub platform: DevicePlatform,
    pub environment: PushEnvironment,
    pub locale: String,
    pub preferences: NotificationPreferences,
    /// `None` keeps the stored token
    pub alert_token: Option<String>,
    /// `None` keeps the stored token
    pub widget_token: Option<String>,
}

/// One document a device has yet to hear about
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingNotification {
    pub device_id: Uuid,
    pub document_id: Id,
    pub kind: NotificationKind,
    pub vault_id: VaultId,
    pub created_at: DateTime<Utc>,
}

/// Rate limiting, backoff and widget bookkeeping for one device
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushDeviceState {
    pub window_started_at: Option<DateTime<Utc>>,
    pub alerts_in_window: u32,
    pub backoff_until: Option<DateTime<Utc>>,
    pub failures: u32,
    pub widgets_dirty_at: Option<DateTime<Utc>>,
    pub widgets_sent_at: Option<DateTime<Utc>>,
    pub test_sent_at: Option<DateTime<Utc>>,
}

/// Validate an APNs token as the app reports it: hexadecimal, of a plausible length
pub fn valid_push_token(token: &str) -> bool {
    (32..=400).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_notify_the_uploader_about_review_and_failure() {
        let prefs = NotificationPreferences::default();
        assert!(prefs.wants(NotificationKind::NeedsReview));
        assert!(prefs.wants(NotificationKind::Failed));
        assert!(!prefs.wants(NotificationKind::Processed));
        assert_eq!(prefs.scope, NotificationScope::Mine);
        assert_eq!(prefs.vaults, VaultSelection::All);
        assert!(!prefs.show_titles);
    }

    #[test]
    fn missing_fields_take_defaults() {
        let prefs: NotificationPreferences = serde_json::from_str(r#"{"processed":true}"#).unwrap();
        assert!(prefs.processed);
        assert!(prefs.needs_review);
    }

    #[test]
    fn vault_selection_round_trips() {
        let vault = VaultId::new();
        for selection in [VaultSelection::All, VaultSelection::Only(vec![vault])] {
            let json = serde_json::to_value(&selection).unwrap();
            assert_eq!(
                serde_json::from_value::<VaultSelection>(json).unwrap(),
                selection
            );
        }
        assert_eq!(
            serde_json::to_value(VaultSelection::All).unwrap(),
            serde_json::json!("all")
        );
        assert!(serde_json::from_str::<VaultSelection>(r#""some""#).is_err());
    }

    #[test]
    fn tokens_must_be_hexadecimal() {
        assert!(valid_push_token(&"ab".repeat(32)));
        assert!(!valid_push_token(&"zz".repeat(32)));
        assert!(!valid_push_token("abcd"));
    }
}
