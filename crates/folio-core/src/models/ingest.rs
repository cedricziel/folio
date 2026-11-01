//! Ingestion sources: places Folio pulls documents from automatically

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{DocumentSource, Id, VaultId};
use crate::error::{Error, Result};

/// Marker shown instead of a stored password
pub const REDACTED: &str = "********";

/// What to do with a consume-folder file once it was imported
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FolderAfterImport {
    /// Remove the file
    Delete,
    /// Move it into `processed/` inside the consume folder
    #[default]
    Move,
}

/// What to do with a mail once its attachments were imported
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MailAfterImport {
    /// Set the `\Seen` flag
    #[default]
    MarkRead,
    /// Move to `move_to`
    Move,
    /// Delete and expunge
    Delete,
}

/// How to reach the mail server
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MailSecurity {
    /// Implicit TLS (usually port 993)
    #[default]
    Tls,
    /// No encryption; only for trusted local networks
    Plain,
}

/// Whether the mail itself becomes a document
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MailBody {
    /// Only attachments are imported
    #[default]
    None,
    /// The whole message is stored as an `.eml` file
    Eml,
}

/// A watched consume folder
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FolderConfig {
    pub path: String,
    #[serde(default = "yes")]
    pub recursive: bool,
    /// Turn sub-folder names into tags: `Insurance/2025/x.pdf` gets `Insurance` and `2025`
    #[serde(default = "yes")]
    pub subfolders_as_tags: bool,
    #[serde(default)]
    pub after_import: FolderAfterImport,
    /// How long a file's size has to stay unchanged before it is imported
    #[serde(default = "default_settle_secs")]
    pub settle_secs: u64,
    /// Full scan interval; the file watcher usually reacts sooner
    #[serde(default = "default_rescan_secs")]
    pub rescan_secs: u64,
}

/// An IMAP account
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EmailConfig {
    pub host: String,
    #[serde(default = "default_imap_port")]
    pub port: u16,
    #[serde(default)]
    pub security: MailSecurity,
    /// Accept self-signed certificates
    #[serde(default)]
    pub accept_invalid_certs: bool,
    pub username: String,
    /// Shown as `********` by the API; send it back unchanged to keep the stored one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Read the password from this file (for Docker secrets); wins over `password`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_file: Option<String>,
    #[serde(default = "default_mailboxes")]
    pub mailboxes: Vec<String>,
    #[serde(default = "default_poll_secs")]
    pub poll_interval_secs: u64,
    /// IMAP SEARCH criteria
    #[serde(default = "default_criteria")]
    pub criteria: String,
    /// Attachment content types to import; `image/*` style wildcards are allowed
    #[serde(default = "default_content_types")]
    pub allowed_content_types: Vec<String>,
    #[serde(default)]
    pub mail_body: MailBody,
    #[serde(default)]
    pub after_import: MailAfterImport,
    /// Target mailbox for `after_import = move`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_to: Option<String>,
}

fn yes() -> bool {
    true
}
fn default_settle_secs() -> u64 {
    5
}
fn default_rescan_secs() -> u64 {
    60
}
fn default_imap_port() -> u16 {
    993
}
fn default_mailboxes() -> Vec<String> {
    vec!["INBOX".to_string()]
}
fn default_poll_secs() -> u64 {
    300
}
fn default_criteria() -> String {
    "UNSEEN".to_string()
}
fn default_content_types() -> Vec<String> {
    ["application/pdf", "image/*"]
        .into_iter()
        .map(String::from)
        .collect()
}

impl EmailConfig {
    /// The effective password: the file's trimmed content, else the inline value
    pub fn resolve_password(&self) -> Result<String> {
        if let Some(path) = &self.password_file {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| Error::Validation(format!("Cannot read password file {path}: {e}")))?;
            return Ok(raw.trim_end_matches(['\r', '\n']).to_string());
        }
        self.password
            .clone()
            .filter(|p| p != REDACTED)
            .ok_or_else(|| Error::Validation("No password configured".to_string()))
    }
}

/// Kind-specific settings of a source
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceConfig {
    Folder(FolderConfig),
    Email(EmailConfig),
}

impl SourceConfig {
    pub fn kind(&self) -> super::SourceKind {
        match self {
            Self::Folder(_) => super::SourceKind::Folder,
            Self::Email(_) => super::SourceKind::Email,
        }
    }

    pub fn validate(&self) -> Result<()> {
        let invalid = |msg: &str| Err(Error::Validation(msg.to_string()));
        match self {
            Self::Folder(f) => {
                if f.path.trim().is_empty() {
                    return invalid("Folder path must not be empty");
                }
            }
            Self::Email(e) => {
                if e.host.trim().is_empty() || e.username.trim().is_empty() {
                    return invalid("Mail host and username must not be empty");
                }
                if e.mailboxes.is_empty() || e.mailboxes.iter().any(|m| m.trim().is_empty()) {
                    return invalid("At least one mailbox is required");
                }
                if e.criteria.contains(['\r', '\n']) {
                    return invalid("Search criteria must be a single line");
                }
                if e.after_import == MailAfterImport::Move
                    && e.move_to.as_deref().is_none_or(|m| m.trim().is_empty())
                {
                    return invalid("after_import = move needs move_to");
                }
            }
        }
        Ok(())
    }

    /// A copy that is safe to show: passwords are replaced by [`REDACTED`]
    pub fn redacted(&self) -> Self {
        match self {
            Self::Email(e) if e.password.is_some() => Self::Email(EmailConfig {
                password: Some(REDACTED.to_string()),
                ..e.clone()
            }),
            other => other.clone(),
        }
    }
}

/// Where a source definition came from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SourceOrigin {
    /// Environment or `config.toml`; read-only in the UI
    Config,
    /// Created in the UI or through the API
    Managed,
}

/// A configured ingestion source
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IngestionSourceDef {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,
    pub name: String,
    pub enabled: bool,
    pub origin: SourceOrigin,
    pub config: SourceConfig,
    /// Where imported documents go; `None` for sources from the configuration,
    /// which use the instance default vault
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub vault_id: Option<VaultId>,
}

/// Fields for creating or replacing a managed source
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IngestionSourceInput {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub config: SourceConfig,
}

impl IngestionSourceInput {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::Validation("Source name must not be empty".into()));
        }
        self.config.validate()
    }
}

/// How the latest run of a source went
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SourceStatus {
    pub last_run_at: Option<DateTime<Utc>>,
    /// Set when the latest run failed; cleared by the next success
    pub last_error: Option<String>,
    pub last_imported: u32,
    pub last_skipped: u32,
    pub last_failed: u32,
}

/// One file or attachment on its way into Folio
#[derive(Debug, Clone)]
pub struct IngestItem {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
    pub source: DocumentSource,
    /// Tag names to attach; missing tags are created
    pub tags: Vec<String>,
}

/// Counts from one poll of a source
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PollReport {
    pub imported: u32,
    pub skipped: u32,
    pub failed: u32,
    /// Asks to be polled again sooner than the regular interval
    pub recheck_in: Option<std::time::Duration>,
}

/// What happened to an item handed to the pipeline
#[derive(Debug, Clone)]
pub enum IngestOutcome {
    Imported(Box<super::Document>),
    /// A pre-ingest rule said no
    Skipped {
        rule: String,
    },
    /// The same content was imported before
    Duplicate,
}

/// Record of something the pipeline already handled
#[derive(Debug, Clone)]
pub struct LedgerEntry {
    /// `hash:<sha256>` or `msg:<source id>:<message id>`
    pub key: String,
    pub source_id: Id,
    pub document_id: Option<Id>,
    pub outcome: String,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_defaults_apply_when_deserializing() {
        let cfg: SourceConfig = serde_json::from_str(
            r#"{"type":"email","host":"imap.test","username":"u","password":"p"}"#,
        )
        .unwrap();
        let SourceConfig::Email(e) = cfg else {
            panic!("email")
        };
        assert_eq!(e.port, 993);
        assert_eq!(e.criteria, "UNSEEN");
        assert_eq!(e.mailboxes, ["INBOX"]);
        assert_eq!(e.after_import, MailAfterImport::MarkRead);
    }

    #[test]
    fn redaction_hides_the_password() {
        let cfg: SourceConfig = serde_json::from_str(
            r#"{"type":"email","host":"h","username":"u","password":"secret"}"#,
        )
        .unwrap();
        let json = serde_json::to_string(&cfg.redacted()).unwrap();
        assert!(!json.contains("secret"));
        assert!(json.contains(REDACTED));
    }

    #[test]
    fn move_without_target_is_invalid() {
        let cfg: SourceConfig = serde_json::from_str(
            r#"{"type":"email","host":"h","username":"u","password":"p","after_import":"move"}"#,
        )
        .unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn password_file_wins_and_is_trimmed() {
        let dir = std::env::temp_dir().join(format!("folio-pw-{}", Id::new_v4()));
        std::fs::write(&dir, "from-file\n").unwrap();
        let cfg: EmailConfig = serde_json::from_value(serde_json::json!({
            "host":"h","username":"u","password":"inline","password_file": dir.to_str().unwrap()
        }))
        .unwrap();
        assert_eq!(cfg.resolve_password().unwrap(), "from-file");
        std::fs::remove_file(dir).unwrap();
    }
}
