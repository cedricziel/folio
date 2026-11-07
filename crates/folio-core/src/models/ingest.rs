//! Ingestion sources: places Folio pulls documents from automatically

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{DocumentSource, Id, UserId, VaultId};
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
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl std::fmt::Debug for EmailConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("security", &self.security)
            .field("accept_invalid_certs", &self.accept_invalid_certs)
            .field("username", &self.username)
            .field("password", &crate::redact::Redacted(&self.password))
            .field("password_file", &self.password_file)
            .field("mailboxes", &self.mailboxes)
            .field("poll_interval_secs", &self.poll_interval_secs)
            .field("criteria", &self.criteria)
            .field("allowed_content_types", &self.allowed_content_types)
            .field("mail_body", &self.mail_body)
            .field("after_import", &self.after_import)
            .field("move_to", &self.move_to)
            .finish()
    }
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

/// A secret from a file such as a Docker secret, without its trailing newline
fn read_secret_file(path: &str, what: &str) -> Result<String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| Error::Validation(format!("Cannot read {what} file {path}: {e}")))?;
    Ok(raw.trim_end_matches(['\r', '\n']).to_string())
}

impl EmailConfig {
    /// The effective password: the file's trimmed content, else the inline value
    pub fn resolve_password(&self) -> Result<String> {
        if let Some(path) = &self.password_file {
            return read_secret_file(path, "password");
        }
        self.password
            .clone()
            .filter(|p| p != REDACTED)
            .ok_or_else(|| Error::Validation("No password configured".to_string()))
    }
}

/// How much of a Paperless library a source keeps in sync
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PaperlessSyncMode {
    /// Import the library once, then stop polling
    Once,
    /// Import documents that are new in Paperless; ignore every other change
    NewOnly,
    /// Paperless is the master: changes, trashing and restores are applied
    #[default]
    Mirror,
}

/// A Paperless-ngx library reached over its REST API
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PaperlessConfig {
    /// Base URL, `http` or `https`
    pub url: String,
    /// Shown as `********` by the API; send it back unchanged to keep the stored one
    /// while the URL stays the same
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Read the token from this file; only accepted in `config.toml`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_file: Option<String>,
    #[serde(default)]
    pub sync_mode: PaperlessSyncMode,
    /// At least 60
    #[serde(default = "default_paperless_poll_secs")]
    pub poll_interval_secs: u64,
    /// ISO 4217 code for monetary values that carry no currency
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_currency: Option<String>,
}

/// The shortest poll interval a Paperless source accepts
pub const PAPERLESS_MIN_POLL_SECS: u64 = 60;

impl std::fmt::Debug for PaperlessConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaperlessConfig")
            .field("url", &self.url)
            .field("token", &crate::redact::Redacted(&self.token))
            .field("token_file", &self.token_file)
            .field("sync_mode", &self.sync_mode)
            .field("poll_interval_secs", &self.poll_interval_secs)
            .field("default_currency", &self.default_currency)
            .finish()
    }
}

impl PaperlessConfig {
    /// The effective token: the file's trimmed content, else the inline value
    pub fn resolve_token(&self) -> Result<String> {
        if let Some(path) = &self.token_file {
            return read_secret_file(path, "token");
        }
        self.token
            .clone()
            .filter(|t| !t.is_empty() && t != REDACTED)
            .ok_or_else(|| Error::Validation("No Paperless token configured".to_string()))
    }

    fn validate(&self) -> Result<()> {
        let invalid = |msg: &str| Err(Error::Validation(msg.to_string()));
        if !url::Url::parse(self.url.trim())
            .is_ok_and(|url| super::metadata::is_http_url_without_credentials(&url))
        {
            return invalid("url must be an http or https URL without credentials");
        }
        if self.poll_interval_secs < PAPERLESS_MIN_POLL_SECS {
            return invalid("poll_interval_secs must be at least 60");
        }
        if self
            .default_currency
            .as_deref()
            .is_some_and(|c| !super::metadata::is_currency_code(c))
        {
            return invalid("default_currency must be a three-letter ISO 4217 code such as EUR");
        }
        if self.token_file.is_none() && self.token.as_deref().is_none_or(str::is_empty) {
            return invalid("token is required");
        }
        Ok(())
    }
}

impl PaperlessSyncMode {
    pub const ALL: [Self; 3] = [Self::Once, Self::NewOnly, Self::Mirror];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::NewOnly => "new_only",
            Self::Mirror => "mirror",
        }
    }

    /// Parse the wire name; unknown values fall back to `mirror`
    pub fn parse(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == value)
            .unwrap_or_default()
    }
}

fn default_paperless_poll_secs() -> u64 {
    900
}

/// Kind-specific settings of a source
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceConfig {
    Folder(FolderConfig),
    Email(EmailConfig),
    Paperless(PaperlessConfig),
}

impl SourceConfig {
    pub fn is_paperless(&self) -> bool {
        matches!(self, Self::Paperless(_))
    }

    pub fn kind(&self) -> super::SourceKind {
        match self {
            Self::Folder(_) => super::SourceKind::Folder,
            Self::Email(_) => super::SourceKind::Email,
            Self::Paperless(_) => super::SourceKind::Paperless,
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
            Self::Paperless(p) => p.validate()?,
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
            Self::Paperless(p) if p.token.is_some() => Self::Paperless(PaperlessConfig {
                token: Some(REDACTED.to_string()),
                ..p.clone()
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

/// Audit metadata retained for managed sources, including sources moved to trash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SourceLifecycle {
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub updated_by: Option<UserId>,
}

/// A configured ingestion source
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IngestionSourceDef {
    /// Configured sources have no persisted lifecycle record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<SourceLifecycle>,
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
    /// The same content has a retained document and a stored original.
    Duplicate,
    /// A pending or incomplete import owns the content; retry without acknowledging the source.
    Busy,
}

/// Record of something the pipeline already handled
#[derive(Debug, Clone)]
pub struct LedgerEntry {
    pub vault_id: VaultId,
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
    fn email_debug_output_hides_the_password() {
        let cfg: SourceConfig = serde_json::from_str(
            r#"{"type":"email","host":"imap.test","username":"u","password":"hunter2"}"#,
        )
        .unwrap();

        let debug = format!("{cfg:?}");

        assert!(!debug.contains("hunter2"), "{debug}");
        assert!(debug.contains("imap.test"));
    }

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

    fn paperless(extra: serde_json::Value) -> SourceConfig {
        let mut body = serde_json::json!({
            "type": "paperless",
            "url": "https://paperless.example",
            "token": "secret-token",
        });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(body).unwrap()
    }

    fn validation_message(cfg: &SourceConfig) -> String {
        match cfg.validate() {
            Err(Error::Validation(message)) => message,
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    #[test]
    fn paperless_defaults_to_mirror_every_fifteen_minutes() {
        let SourceConfig::Paperless(p) = paperless(serde_json::json!({})) else {
            panic!("paperless")
        };
        assert_eq!(p.sync_mode, PaperlessSyncMode::Mirror);
        assert_eq!(p.poll_interval_secs, 900);
        assert_eq!(p.default_currency, None);
        assert_eq!(
            SourceConfig::Paperless(p).kind(),
            crate::models::SourceKind::Paperless
        );
    }

    #[test]
    fn paperless_sync_modes_use_snake_case() {
        for (wire, mode) in [
            ("once", PaperlessSyncMode::Once),
            ("new_only", PaperlessSyncMode::NewOnly),
            ("mirror", PaperlessSyncMode::Mirror),
        ] {
            let SourceConfig::Paperless(p) = paperless(serde_json::json!({ "sync_mode": wire }))
            else {
                panic!("paperless")
            };
            assert_eq!(p.sync_mode, mode);
        }
    }

    #[test]
    fn paperless_accepts_http_and_https() {
        for url in [
            "http://paperless.lan:8000",
            "https://paperless.example/sub/",
        ] {
            paperless(serde_json::json!({ "url": url }))
                .validate()
                .unwrap();
        }
    }

    #[test]
    fn paperless_rejects_other_url_schemes_naming_the_field() {
        for url in [
            "ftp://paperless.example",
            "file:///etc/passwd",
            "paperless.example",
            "",
        ] {
            let message = validation_message(&paperless(serde_json::json!({ "url": url })));
            assert!(message.contains("url"), "{url}: {message}");
        }
    }

    #[test]
    fn paperless_rejects_urls_with_credentials() {
        let cfg = paperless(serde_json::json!({ "url": "https://user:pw@paperless.example" }));
        assert!(validation_message(&cfg).contains("url"));
    }

    #[test]
    fn paperless_poll_interval_must_be_at_least_a_minute() {
        let message =
            validation_message(&paperless(serde_json::json!({ "poll_interval_secs": 59 })));
        assert!(message.contains("poll_interval_secs"), "{message}");
        paperless(serde_json::json!({ "poll_interval_secs": 60 }))
            .validate()
            .unwrap();
    }

    #[test]
    fn paperless_default_currency_is_a_three_letter_code() {
        paperless(serde_json::json!({ "default_currency": "EUR" }))
            .validate()
            .unwrap();
        for bad in ["EU", "EURO", "eu1"] {
            let message =
                validation_message(&paperless(serde_json::json!({ "default_currency": bad })));
            assert!(message.contains("default_currency"), "{bad}: {message}");
        }
    }

    #[test]
    fn paperless_needs_a_token_or_a_token_file() {
        let cfg: SourceConfig = serde_json::from_value(serde_json::json!({
            "type": "paperless", "url": "https://paperless.example"
        }))
        .unwrap();
        assert!(validation_message(&cfg).contains("token"));
    }

    #[test]
    fn paperless_token_is_redacted_and_hidden_from_debug_output() {
        let cfg = paperless(serde_json::json!({}));
        assert!(!format!("{cfg:?}").contains("secret-token"));
        let json = serde_json::to_string(&cfg.redacted()).unwrap();
        assert!(!json.contains("secret-token"), "{json}");
        assert!(json.contains(REDACTED));
    }

    #[test]
    fn paperless_token_file_wins_and_is_trimmed() {
        let path = std::env::temp_dir().join(format!("folio-pl-{}", Id::new_v4()));
        std::fs::write(&path, "from-file\n").unwrap();
        let SourceConfig::Paperless(p) =
            paperless(serde_json::json!({ "token_file": path.to_str().unwrap() }))
        else {
            panic!("paperless")
        };
        assert_eq!(p.resolve_token().unwrap(), "from-file");
        std::fs::remove_file(path).unwrap();
    }
}
