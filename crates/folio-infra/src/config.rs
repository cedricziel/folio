//! Configuration management
//!
//! Precedence, lowest to highest: built-in defaults, `config.toml`, environment
//! variables. Settings that the database manages (see `/settings`) override all
//! of these at runtime.
//!
//! Nested keys are addressed in the environment with a double underscore:
//! `FOLIO_AI__OPENAI_API_KEY` sets `ai.openai_api_key`. Any variable can also be
//! supplied through a file with the `_FILE` suffix
//! (`FOLIO_AI__OPENAI_API_KEY_FILE=/run/secrets/openai`), which suits Docker and
//! Kubernetes secrets.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::PathBuf;

use figment::{
    providers::{Format, Serialized, Toml},
    Figment,
};
use folio_core::models::{
    AppleAppId, EmailConfig, EmbeddingSettings, FolderConfig, Id, IngestionSourceDef, OcrSettings,
    PaperlessConfig, ProcessingSettings, SourceConfig, SourceOrigin, VaultId,
};
use folio_core::redact::Redacted;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const ENV_PREFIX: &str = "FOLIO_";
const FILE_SUFFIX: &str = "_FILE";
const CONFIG_PATH_ENV: &str = "FOLIO_CONFIG";
const DEFAULT_CONFIG_PATH: &str = "config.toml";

/// Application configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub storage: StorageConfig,
    pub ai: AiConfig,
    pub ocr: OcrSettings,
    pub embeddings: EmbeddingSettings,
    pub processing: ProcessingSettings,
    pub ingest: IngestConfig,
    pub auth: AuthConfig,
    pub mcp: McpConfig,
    pub import: ImportConfig,
    pub apple: AppleConfig,
}

/// Automatic ingestion, configured per named source
///
/// ```toml
/// [ingest.folders.inbox]
/// path = "/consume"
///
/// [ingest.email.work]
/// host = "imap.example.com"
/// username = "me"
/// password_file = "/run/secrets/imap_work"
///
/// [ingest.paperless.home]
/// url = "https://paperless.example"
/// token_file = "/run/secrets/paperless_token"
/// vault = "<vault ID>"
/// ```
///
/// The same keys work as environment variables, for example
/// `FOLIO_INGEST__FOLDERS__INBOX__PATH=/consume`. See `docs/ingestion.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestConfig {
    pub folders: BTreeMap<String, Configured<FolderConfig>>,
    pub email: BTreeMap<String, Configured<EmailConfig>>,
    pub paperless: BTreeMap<String, ConfiguredPaperless>,
}

/// A configured Paperless library; unlike folders and mailboxes it names its vault
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfiguredPaperless {
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// Required; checked by [`IngestConfig::check`] so the error can name the section
    #[serde(default)]
    pub vault: Option<VaultId>,
    #[serde(flatten)]
    pub settings: PaperlessConfig,
}

/// A configured source with an on/off switch next to its settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Configured<T> {
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(flatten)]
    pub settings: T,
}

fn enabled_by_default() -> bool {
    true
}

/// A stable id, so a source keeps its status across restarts
fn config_source_id(kind: &str, name: &str) -> Id {
    let digest = Sha256::digest(format!("folio-config-source:{kind}:{name}"));
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Id::from_bytes(bytes)
}

impl IngestConfig {
    /// The configured sources as read-only definitions
    pub fn definitions(&self) -> Vec<IngestionSourceDef> {
        let folders = self.folders.iter().map(|(name, c)| IngestionSourceDef {
            lifecycle: None,
            id: config_source_id("folder", name),
            name: name.clone(),
            enabled: c.enabled,
            origin: SourceOrigin::Config,
            config: SourceConfig::Folder(c.settings.clone()),
            vault_id: None,
        });
        let mailboxes = self.email.iter().map(|(name, c)| IngestionSourceDef {
            lifecycle: None,
            id: config_source_id("email", name),
            name: name.clone(),
            enabled: c.enabled,
            origin: SourceOrigin::Config,
            config: SourceConfig::Email(c.settings.clone()),
            vault_id: None,
        });
        let paperless = self.paperless.iter().map(|(name, c)| IngestionSourceDef {
            lifecycle: None,
            id: config_source_id("paperless", name),
            name: name.clone(),
            enabled: c.enabled,
            origin: SourceOrigin::Config,
            config: SourceConfig::Paperless(c.settings.clone()),
            vault_id: c.vault,
        });
        folders.chain(mailboxes).chain(paperless).collect()
    }

    /// Paperless sections need a vault, valid settings and a vault of their own
    pub fn check(&self) -> Result<(), String> {
        let mut vaults = HashMap::new();
        for (name, source) in &self.paperless {
            let section = format!("[ingest.paperless.{name}]");
            let vault = source
                .vault
                .ok_or_else(|| format!("{section} needs a vault: set vault = \"<vault ID>\""))?;
            SourceConfig::Paperless(source.settings.clone())
                .validate()
                .map_err(|e| format!("{section}: {e}"))?;
            if let Some(other) = vaults.insert(vault, name) {
                return Err(format!(
                    "{section} uses the same vault as [ingest.paperless.{other}]; \
                     a vault has at most one Paperless source"
                ));
            }
        }
        Ok(())
    }
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// Externally visible base URL (for example `https://folio.example.com`).
    /// Used in OAuth metadata, redirects and webhook payload links; derived from the
    /// request or `http://<host>:<port>` when unset.
    pub public_url: Option<String>,
}

impl ServerConfig {
    /// Base URL other systems can use to reach this instance
    pub fn base_url(&self) -> String {
        match self.public_url.as_deref().filter(|u| !u.is_empty()) {
            Some(url) => url.trim_end_matches('/').to_string(),
            None => format!("http://{}:{}", self.host, self.port),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3000,
            public_url: None,
        }
    }
}

impl ServerConfig {
    /// Whether the listener is only reachable from this machine.
    pub fn is_loopback(&self) -> bool {
        let host = self.host.trim_matches(|c| c == '[' || c == ']');
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    }
}

/// Database configuration
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    pub url: String,
}

impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseConfig")
            .field("url", &"<redacted>")
            .finish()
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "sqlite:folio.db?mode=rwc".to_string(),
        }
    }
}

/// Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    pub backend: StorageBackend,
    /// Root directory for the `local` backend.
    pub path: String,
    pub s3: S3Config,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: StorageBackend::default(),
            path: "./data".to_string(),
            s3: S3Config::default(),
        }
    }
}

/// Where document files live
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    #[default]
    Local,
    S3,
}

/// S3-compatible object storage (AWS S3, MinIO, Garage, R2, ...)
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct S3Config {
    pub bucket: Option<String>,
    pub region: Option<String>,
    /// Custom endpoint for non-AWS services.
    pub endpoint: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    /// Address buckets as `endpoint/bucket` instead of `bucket.endpoint`.
    /// Unset means path-style when a custom endpoint is set, virtual-hosted
    /// style otherwise.
    pub path_style: Option<bool>,
    /// Allow plain HTTP endpoints.
    pub allow_http: bool,
    /// Key prefix inside the bucket.
    pub prefix: Option<String>,
}

impl fmt::Debug for S3Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Config")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .field("access_key_id", &Redacted(&self.access_key_id))
            .field("secret_access_key", &Redacted(&self.secret_access_key))
            .field("path_style", &self.path_style)
            .field("allow_http", &self.allow_http)
            .field("prefix", &self.prefix)
            .finish()
    }
}

/// Authentication configuration
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    /// Require a session or token for the web UI and API.
    pub required: bool,
    pub forward: ForwardAuthConfig,
    pub oidc: OidcConfig,
    /// One-time secret for `/setup` while no user exists. A random one is
    /// generated and logged at startup when unset.
    pub setup_token: Option<String>,
    /// Encrypts TOTP seeds at rest. When unset a key is generated on first
    /// start and kept in the database settings.
    pub secret_key: Option<String>,
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("required", &self.required)
            .field("forward", &self.forward)
            .field("setup_token", &Redacted(&self.setup_token))
            .field("secret_key", &Redacted(&self.secret_key))
            .finish()
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            required: true,
            forward: ForwardAuthConfig::default(),
            oidc: OidcConfig::default(),
            setup_token: None,
            secret_key: None,
        }
    }
}

/// Trusted-header authentication behind a reverse proxy or SSO gateway.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ForwardAuthConfig {
    pub enabled: bool,
    pub user_header: String,
    pub email_header: String,
    pub name_header: String,
    /// Empty (the default) means group headers are ignored.
    pub groups_header: String,
    /// Empty (the default) means role headers are ignored.
    pub role_header: String,
    /// Empty (the default) links accounts by the user header.
    pub user_id_header: String,
    /// CIDRs or addresses of the proxies whose headers are honoured.
    pub trusted_proxies: Vec<String>,
    /// Members of this (external) group become admins; needs `groups_header`.
    pub admin_group: Option<String>,
    /// Users with this role become admins; needs `role_header`.
    pub admin_role: Option<String>,
    /// When set, the proxy must also send it in `secret_header`.
    pub shared_secret: Option<String>,
    pub secret_header: String,
    /// Lifetime of sessions created by forward-auth logins, in seconds.
    pub session_ttl_secs: u64,
    /// Separator between group names in `groups_header`.
    pub group_separator: String,
}

impl fmt::Debug for ForwardAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForwardAuthConfig")
            .field("enabled", &self.enabled)
            .field("user_header", &self.user_header)
            .field("email_header", &self.email_header)
            .field("name_header", &self.name_header)
            .field("groups_header", &self.groups_header)
            .field("role_header", &self.role_header)
            .field("user_id_header", &self.user_id_header)
            .field("trusted_proxies", &self.trusted_proxies)
            .field("admin_group", &self.admin_group)
            .field("admin_role", &self.admin_role)
            .field("shared_secret", &Redacted(&self.shared_secret))
            .field("secret_header", &self.secret_header)
            .field("session_ttl_secs", &self.session_ttl_secs)
            .field("group_separator", &self.group_separator)
            .finish()
    }
}

impl Default for ForwardAuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            user_header: "Remote-User".to_string(),
            email_header: "Remote-Email".to_string(),
            name_header: "Remote-Name".to_string(),
            groups_header: String::new(),
            role_header: String::new(),
            user_id_header: String::new(),
            trusted_proxies: Vec::new(),
            admin_group: None,
            admin_role: None,
            shared_secret: None,
            secret_header: "X-Folio-Proxy-Secret".to_string(),
            session_ttl_secs: 12 * 3600,
            group_separator: ",".to_string(),
        }
    }
}

/// Sign-in through an OpenID Connect identity provider.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OidcConfig {
    pub enabled: bool,
    pub issuer: Option<String>,
    pub client_id: Option<String>,
    /// Unset for public clients.
    pub client_secret: Option<String>,
    /// Space-separated scopes to request.
    pub scopes: String,
    /// Label of the sign-in button.
    pub display_name: String,
    /// Claim holding the user's group names.
    pub groups_claim: String,
    /// Members of this (external) group become admins.
    pub admin_group: Option<String>,
    /// Keep the username and password form next to the SSO button.
    pub allow_local_login: bool,
    /// Create an account the first time someone signs in.
    pub auto_create_users: bool,
}

impl std::fmt::Debug for OidcConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcConfig")
            .field("enabled", &self.enabled)
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "***"))
            .finish_non_exhaustive()
    }
}

impl Default for OidcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            issuer: None,
            client_id: None,
            client_secret: None,
            scopes: "openid profile email".to_string(),
            display_name: "Sign in with SSO".to_string(),
            groups_claim: "groups".to_string(),
            admin_group: None,
            allow_local_login: true,
            auto_create_users: true,
        }
    }
}

/// AI provider configuration
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub provider: String,
    pub openai_api_key: Option<String>,
    pub anthropic_api_key: Option<String>,
}

impl fmt::Debug for AiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiConfig")
            .field("provider", &self.provider)
            .field("openai_api_key", &Redacted(&self.openai_api_key))
            .field("anthropic_api_key", &Redacted(&self.anthropic_api_key))
            .finish()
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: "openai".to_string(),
            openai_api_key: None,
            anthropic_api_key: None,
        }
    }
}

/// MCP server configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    /// Serve the MCP endpoint at `/mcp`. Unset means on when `auth.required` is
    /// true and off otherwise, since the endpoint is only protected by auth.
    /// The `mcp.enabled` DB setting overrides this.
    pub enabled: Option<bool>,
    /// Extra `Host` header values to accept besides loopback
    pub allowed_hosts: Vec<String>,
}

/// Apple app IDs whose apps may open this server's web links (Universal Links)
///
/// The `apple.app_ids` DB setting replaces this list when it is non-empty.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppleConfig {
    /// `TEAMID.bundle.id` entries; empty serves no app site association.
    pub app_ids: Vec<String>,
}

impl AppleConfig {
    /// The validated app IDs, without repeats
    pub fn parsed(&self) -> Result<Vec<AppleAppId>, String> {
        AppleAppId::parse_all(self.app_ids.iter().map(String::as_str))
            .map_err(|e| format!("apple.app_ids (FOLIO_APPLE__APP_IDS): {e}"))
    }
}

/// Server directories an import may read, and the size limit for one file
///
/// Config-only: which server paths the app reads is a deployment decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportConfig {
    /// Empty disables imports.
    pub allowed_roots: Vec<PathBuf>,
    pub max_file_bytes: u64,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            allowed_roots: Vec::new(),
            max_file_bytes: 512 * 1024 * 1024,
        }
    }
}

impl ImportConfig {
    pub fn imports_enabled(&self) -> bool {
        !self.allowed_roots.is_empty()
    }
}

/// How an environment value is converted before it reaches the config tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Str,
    Port,
    /// A non-negative whole number
    Int,
    Bool,
    /// Comma-separated values
    List,
}

/// One configurable setting. This table drives env parsing and keeps
/// `docs/configuration.md` honest (a test fails when they drift).
#[derive(Debug)]
pub struct Setting {
    /// Dotted path in `config.toml`.
    pub key: &'static str,
    /// Canonical environment variable.
    pub env: &'static str,
    pub kind: ValueKind,
}

pub const SETTINGS: &[Setting] = &[
    Setting {
        key: "server.host",
        env: "FOLIO_SERVER__HOST",
        kind: ValueKind::Str,
    },
    Setting {
        key: "server.port",
        env: "FOLIO_SERVER__PORT",
        kind: ValueKind::Port,
    },
    Setting {
        key: "server.public_url",
        env: "FOLIO_SERVER__PUBLIC_URL",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.required",
        env: "FOLIO_AUTH__REQUIRED",
        kind: ValueKind::Bool,
    },
    Setting {
        key: "auth.forward.enabled",
        env: "FOLIO_AUTH__FORWARD__ENABLED",
        kind: ValueKind::Bool,
    },
    Setting {
        key: "auth.forward.user_header",
        env: "FOLIO_AUTH__FORWARD__USER_HEADER",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.forward.email_header",
        env: "FOLIO_AUTH__FORWARD__EMAIL_HEADER",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.forward.name_header",
        env: "FOLIO_AUTH__FORWARD__NAME_HEADER",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.forward.groups_header",
        env: "FOLIO_AUTH__FORWARD__GROUPS_HEADER",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.forward.role_header",
        env: "FOLIO_AUTH__FORWARD__ROLE_HEADER",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.forward.user_id_header",
        env: "FOLIO_AUTH__FORWARD__USER_ID_HEADER",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.forward.trusted_proxies",
        env: "FOLIO_AUTH__FORWARD__TRUSTED_PROXIES",
        kind: ValueKind::List,
    },
    Setting {
        key: "auth.forward.admin_group",
        env: "FOLIO_AUTH__FORWARD__ADMIN_GROUP",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.forward.admin_role",
        env: "FOLIO_AUTH__FORWARD__ADMIN_ROLE",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.forward.shared_secret",
        env: "FOLIO_AUTH__FORWARD__SHARED_SECRET",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.forward.secret_header",
        env: "FOLIO_AUTH__FORWARD__SECRET_HEADER",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.forward.session_ttl_secs",
        env: "FOLIO_AUTH__FORWARD__SESSION_TTL_SECS",
        kind: ValueKind::Int,
    },
    Setting {
        key: "auth.forward.group_separator",
        env: "FOLIO_AUTH__FORWARD__GROUP_SEPARATOR",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.setup_token",
        env: "FOLIO_AUTH__SETUP_TOKEN",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.secret_key",
        env: "FOLIO_AUTH__SECRET_KEY",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.oidc.enabled",
        env: "FOLIO_AUTH__OIDC__ENABLED",
        kind: ValueKind::Bool,
    },
    Setting {
        key: "auth.oidc.issuer",
        env: "FOLIO_AUTH__OIDC__ISSUER",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.oidc.client_id",
        env: "FOLIO_AUTH__OIDC__CLIENT_ID",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.oidc.client_secret",
        env: "FOLIO_AUTH__OIDC__CLIENT_SECRET",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.oidc.scopes",
        env: "FOLIO_AUTH__OIDC__SCOPES",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.oidc.display_name",
        env: "FOLIO_AUTH__OIDC__DISPLAY_NAME",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.oidc.groups_claim",
        env: "FOLIO_AUTH__OIDC__GROUPS_CLAIM",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.oidc.admin_group",
        env: "FOLIO_AUTH__OIDC__ADMIN_GROUP",
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.oidc.allow_local_login",
        env: "FOLIO_AUTH__OIDC__ALLOW_LOCAL_LOGIN",
        kind: ValueKind::Bool,
    },
    Setting {
        key: "auth.oidc.auto_create_users",
        env: "FOLIO_AUTH__OIDC__AUTO_CREATE_USERS",
        kind: ValueKind::Bool,
    },
    Setting {
        key: "database.url",
        env: "FOLIO_DATABASE__URL",
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.backend",
        env: "FOLIO_STORAGE__BACKEND",
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.path",
        env: "FOLIO_STORAGE__PATH",
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.bucket",
        env: "FOLIO_STORAGE__S3__BUCKET",
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.region",
        env: "FOLIO_STORAGE__S3__REGION",
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.endpoint",
        env: "FOLIO_STORAGE__S3__ENDPOINT",
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.access_key_id",
        env: "FOLIO_STORAGE__S3__ACCESS_KEY_ID",
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.secret_access_key",
        env: "FOLIO_STORAGE__S3__SECRET_ACCESS_KEY",
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.path_style",
        env: "FOLIO_STORAGE__S3__PATH_STYLE",
        kind: ValueKind::Bool,
    },
    Setting {
        key: "storage.s3.allow_http",
        env: "FOLIO_STORAGE__S3__ALLOW_HTTP",
        kind: ValueKind::Bool,
    },
    Setting {
        key: "storage.s3.prefix",
        env: "FOLIO_STORAGE__S3__PREFIX",
        kind: ValueKind::Str,
    },
    Setting {
        key: "ai.provider",
        env: "FOLIO_AI__PROVIDER",
        kind: ValueKind::Str,
    },
    Setting {
        key: "ai.openai_api_key",
        env: "FOLIO_AI__OPENAI_API_KEY",
        kind: ValueKind::Str,
    },
    Setting {
        key: "ai.anthropic_api_key",
        env: "FOLIO_AI__ANTHROPIC_API_KEY",
        kind: ValueKind::Str,
    },
    Setting {
        key: "ocr.provider",
        env: "FOLIO_OCR__PROVIDER",
        kind: ValueKind::Str,
    },
    Setting {
        key: "ocr.model",
        env: "FOLIO_OCR__MODEL",
        kind: ValueKind::Str,
    },
    Setting {
        key: "ocr.languages",
        env: "FOLIO_OCR__LANGUAGES",
        kind: ValueKind::Str,
    },
    Setting {
        key: "ocr.base_url",
        env: "FOLIO_OCR__BASE_URL",
        kind: ValueKind::Str,
    },
    Setting {
        key: "embeddings.provider",
        env: "FOLIO_EMBEDDINGS__PROVIDER",
        kind: ValueKind::Str,
    },
    Setting {
        key: "embeddings.model",
        env: "FOLIO_EMBEDDINGS__MODEL",
        kind: ValueKind::Str,
    },
    Setting {
        key: "embeddings.base_url",
        env: "FOLIO_EMBEDDINGS__BASE_URL",
        kind: ValueKind::Str,
    },
    Setting {
        key: "processing.auto_ocr",
        env: "FOLIO_PROCESSING__AUTO_OCR",
        kind: ValueKind::Bool,
    },
    Setting {
        key: "processing.auto_thumbnail",
        env: "FOLIO_PROCESSING__AUTO_THUMBNAIL",
        kind: ValueKind::Bool,
    },
    Setting {
        key: "processing.auto_embed",
        env: "FOLIO_PROCESSING__AUTO_EMBED",
        kind: ValueKind::Bool,
    },
    Setting {
        key: "mcp.enabled",
        env: "FOLIO_MCP__ENABLED",
        kind: ValueKind::Bool,
    },
    Setting {
        key: "mcp.allowed_hosts",
        env: "FOLIO_MCP__ALLOWED_HOSTS",
        kind: ValueKind::List,
    },
    Setting {
        key: "import.allowed_roots",
        env: "FOLIO_IMPORT__ALLOWED_ROOTS",
        kind: ValueKind::List,
    },
    Setting {
        key: "import.max_file_bytes",
        env: "FOLIO_IMPORT__MAX_FILE_BYTES",
        kind: ValueKind::Int,
    },
    Setting {
        key: "apple.app_ids",
        env: "FOLIO_APPLE__APP_IDS",
        kind: ValueKind::List,
    },
];

impl Config {
    /// Rejects combinations that are unsafe to run. Called once at startup.
    pub fn validate(&self) -> Result<(), String> {
        self.apple.parsed()?;
        if let Some(url) = self.server.public_url.as_deref().filter(|u| !u.is_empty()) {
            url::Url::parse(url).map_err(|e| {
                format!("server.public_url (FOLIO_SERVER__PUBLIC_URL) is not a valid URL ({e}): {url:?}")
            })?;
        }
        if !self.auth.required {
            return Ok(());
        }
        match self.server.public_url.as_deref().filter(|u| !u.is_empty()) {
            Some(url) if !(url.starts_with("https://") || url.starts_with("http://")) => Err(
                format!("server.public_url must start with http:// or https://, got {url:?}"),
            ),
            Some(_) => Ok(()),
            None if self.server.is_loopback() => Ok(()),
            None => Err(format!(
                "server.public_url (FOLIO_SERVER__PUBLIC_URL) is required when auth is enabled \
                 and the server listens on {}: tokens, OAuth metadata and cookie flags are \
                 derived from it, never from request headers. Set it to the externally visible \
                 URL, for example https://folio.example.com",
                self.server.host
            )),
        }
    }

    /// Load configuration from `config.toml` (or the file named by
    /// `FOLIO_CONFIG`) and the process environment.
    #[allow(clippy::result_large_err)]
    pub fn load() -> Result<Self, figment::Error> {
        // Load .env file if present (for development)
        let _ = dotenvy::dotenv();

        let path =
            std::env::var(CONFIG_PATH_ENV).unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());
        Self::load_with(&path, std::env::vars())
    }

    /// Load configuration from a specific file path and the process environment.
    #[allow(clippy::result_large_err)]
    pub fn load_from(path: &str) -> Result<Self, figment::Error> {
        let _ = dotenvy::dotenv();
        Self::load_with(path, std::env::vars())
    }

    /// Load configuration from a file and an explicit set of environment
    /// variables. A missing file is not an error.
    #[allow(clippy::result_large_err)]
    pub fn load_with(
        path: &str,
        vars: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, figment::Error> {
        let vars: HashMap<String, String> = vars.into_iter().collect();
        let mut figment =
            Figment::from(Serialized::defaults(Config::default())).merge(Toml::file(path));

        for setting in SETTINGS {
            if let Some(value) = read_var(setting.env, &vars)? {
                figment =
                    figment.merge(Serialized::default(setting.key, to_value(setting, value)?));
            }
        }
        if let Some(ingest) = ingest_from_env(&vars)? {
            figment = figment.merge(Serialized::default("ingest", ingest));
        }
        let config: Self = figment.extract()?;
        config.ingest.check().map_err(figment::Error::from)?;
        Ok(config)
    }
}

const INGEST_ENV_PREFIX: &str = "FOLIO_INGEST__";

/// Collect `FOLIO_INGEST__<FOLDERS|EMAIL|PAPERLESS>__<NAME>__<KEY>` variables into a tree
///
/// Sources have arbitrary names, so they cannot be listed in [`SETTINGS`].
/// Values are read as bool, integer, `[a, b]` list, or text. A `_FILE` suffix
/// reads the value from a file, like everywhere else.
#[allow(clippy::result_large_err)]
fn ingest_from_env(
    vars: &HashMap<String, String>,
) -> Result<Option<serde_json::Value>, figment::Error> {
    let mut names: Vec<_> = vars
        .keys()
        .filter(|n| n.starts_with(INGEST_ENV_PREFIX))
        .collect();
    names.sort();

    let mut root = serde_json::Map::new();
    for name in names {
        let (base, from_file) = match name.strip_suffix(FILE_SUFFIX) {
            Some(base) => (base, true),
            None => (name.as_str(), false),
        };
        if from_file && vars.contains_key(base) {
            return Err(figment::Error::from(format!(
                "{base} and {name} are both set; use only one"
            )));
        }
        let raw = read_var(base, vars)?.unwrap_or_default();

        let path: Vec<String> = base[INGEST_ENV_PREFIX.len()..]
            .split("__")
            .map(str::to_ascii_lowercase)
            .collect();
        let [kind, source, key] = path.as_slice() else {
            return Err(figment::Error::from(format!(
                "{base}: expected FOLIO_INGEST__<FOLDERS|EMAIL|PAPERLESS>__<NAME>__<KEY>"
            )));
        };
        root.entry(kind.clone())
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .expect("created as an object")
            .entry(source.clone())
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .expect("created as an object")
            .insert(key.clone(), parse_loose(&raw));
    }
    Ok((!root.is_empty()).then_some(serde_json::Value::Object(root)))
}

fn parse_loose(raw: &str) -> serde_json::Value {
    let raw = raw.trim();
    if let Some(inner) = raw.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        return inner
            .split(',')
            .map(|item| item.trim().trim_matches(['"', '\'']))
            .filter(|item| !item.is_empty())
            .map(serde_json::Value::from)
            .collect();
    }
    match raw.to_ascii_lowercase().as_str() {
        "true" => return serde_json::Value::Bool(true),
        "false" => return serde_json::Value::Bool(false),
        _ => {}
    }
    raw.parse::<i64>()
        .map(serde_json::Value::from)
        .unwrap_or_else(|_| serde_json::Value::from(raw))
}

#[allow(clippy::result_large_err)]
fn read_var(name: &str, vars: &HashMap<String, String>) -> Result<Option<String>, figment::Error> {
    debug_assert!(name.starts_with(ENV_PREFIX));
    let direct = vars.get(name);
    let file = vars.get(&format!("{name}{FILE_SUFFIX}"));
    match (direct, file) {
        (Some(_), Some(_)) => Err(figment::Error::from(format!(
            "{name} and {name}{FILE_SUFFIX} are both set; use only one"
        ))),
        (Some(value), None) => Ok(Some(value.clone())),
        (None, Some(path)) => std::fs::read_to_string(path)
            .map(|s| Some(s.trim_end_matches(['\r', '\n']).to_string()))
            .map_err(|e| {
                figment::Error::from(format!("failed to read {name}{FILE_SUFFIX} ({path}): {e}"))
            }),
        (None, None) => Ok(None),
    }
}

#[allow(clippy::result_large_err)]
fn to_value(setting: &Setting, raw: String) -> Result<figment::value::Value, figment::Error> {
    use figment::value::Value;
    let invalid = |expected: &str| {
        figment::Error::from(format!("{}: expected {expected}, got {raw:?}", setting.env))
    };
    match setting.kind {
        ValueKind::Str => Ok(Value::from(raw)),
        ValueKind::Port => raw
            .trim()
            .parse::<u16>()
            .map(Value::from)
            .map_err(|_| invalid("a port number")),
        ValueKind::Int => raw
            .trim()
            .parse::<u64>()
            .map(Value::from)
            .map_err(|_| invalid("a whole number")),
        ValueKind::List => {
            let items: Vec<&str> = raw
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            Value::serialize(items).map_err(|e| invalid(&format!("a comma-separated list ({e})")))
        }
        ValueKind::Bool => match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(Value::from(true)),
            "false" | "0" | "no" | "off" => Ok(Value::from(false)),
            _ => Err(invalid("a boolean")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_contains_secrets() {
        let config = Config {
            database: DatabaseConfig {
                url: "postgres://folio:db-secret@db/folio".into(),
            },
            storage: StorageConfig {
                s3: S3Config {
                    bucket: Some("docs".into()),
                    access_key_id: Some("key-id-secret".into()),
                    secret_access_key: Some("access-secret".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
            auth: AuthConfig {
                setup_token: Some("setup-secret".into()),
                secret_key: Some("totp-key-secret".into()),
                forward: ForwardAuthConfig {
                    shared_secret: Some("proxy-secret".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ai: AiConfig {
                provider: "openai".into(),
                openai_api_key: Some("sk-openai-secret".into()),
                anthropic_api_key: Some("sk-anthropic-secret".into()),
            },
            ..Default::default()
        };

        let debug = format!("{config:?}");

        for secret in [
            "db-secret",
            "key-id-secret",
            "access-secret",
            "setup-secret",
            "totp-key-secret",
            "proxy-secret",
            "sk-openai-secret",
            "sk-anthropic-secret",
        ] {
            assert!(!debug.contains(secret), "{secret} leaked: {debug}");
        }
        assert!(debug.contains("docs"), "non-secret fields stay visible");
        assert!(debug.contains("<redacted>"));
    }

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn load(pairs: &[(&str, &str)]) -> Config {
        Config::load_with("/nonexistent/config.toml", env(pairs)).unwrap()
    }

    fn lookup(config: &Config, key: &str) -> Option<String> {
        let json = serde_json::to_value(config).unwrap();
        let mut node = &json;
        for part in key.split('.') {
            node = node.get(part)?;
        }
        match node {
            serde_json::Value::Null => None,
            serde_json::Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        }
    }

    #[test]
    fn ingest_sources_load_from_nested_environment_variables() {
        let secret = tempfile::tempdir().unwrap();
        let pw = secret.path().join("pw");
        std::fs::write(&pw, "from-file\n").unwrap();
        let config = load(&[
            ("FOLIO_INGEST__FOLDERS__INBOX__PATH", "/consume"),
            ("FOLIO_INGEST__FOLDERS__INBOX__SETTLE_SECS", "2"),
            ("FOLIO_INGEST__EMAIL__WORK__HOST", "imap.test"),
            ("FOLIO_INGEST__EMAIL__WORK__USERNAME", "me"),
            (
                "FOLIO_INGEST__EMAIL__WORK__PASSWORD_FILE",
                pw.to_str().unwrap(),
            ),
            (
                "FOLIO_INGEST__EMAIL__WORK__MAILBOXES",
                "[INBOX, \"Invoices\"]",
            ),
            ("FOLIO_INGEST__EMAIL__OLD__HOST", "x"),
            ("FOLIO_INGEST__EMAIL__OLD__USERNAME", "x"),
            ("FOLIO_INGEST__EMAIL__OLD__PASSWORD", "x"),
            ("FOLIO_INGEST__EMAIL__OLD__ENABLED", "false"),
        ]);
        let defs = config.ingest.definitions();
        assert_eq!(defs.len(), 3);
        let SourceConfig::Folder(f) = &defs.iter().find(|d| d.name == "inbox").unwrap().config
        else {
            panic!("folder")
        };
        assert_eq!(
            (f.path.as_str(), f.settle_secs, f.recursive),
            ("/consume", 2, true)
        );
        let SourceConfig::Email(e) = &defs.iter().find(|d| d.name == "work").unwrap().config else {
            panic!("email")
        };
        assert_eq!(e.mailboxes, ["INBOX", "Invoices"]);
        assert_eq!(e.password.as_deref(), Some("from-file"));
        assert!(!defs.iter().find(|d| d.name == "old").unwrap().enabled);
        assert_eq!(defs[0].id, config.ingest.definitions()[0].id);
    }

    #[allow(clippy::result_large_err)]
    fn load_toml(toml: &str) -> Result<Config, figment::Error> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, toml).unwrap();
        Config::load_with(path.to_str().unwrap(), env(&[]))
    }

    const VAULT: &str = "0190d0d2-6c4b-7d3e-9c35-000000000001";

    #[test]
    fn paperless_sources_load_from_config_toml_into_their_vault() {
        let config = load_toml(&format!(
            r#"
[ingest.paperless.home]
url = "https://paperless.example"
token_file = "/run/secrets/paperless_token"
vault = "{VAULT}"
sync_mode = "new_only"
poll_interval_secs = 600
default_currency = "EUR"
"#
        ))
        .unwrap();
        let defs = config.ingest.definitions();
        let [def] = defs.as_slice() else {
            panic!("one source: {defs:?}")
        };
        assert_eq!(def.name, "home");
        assert_eq!(def.origin, SourceOrigin::Config);
        assert_eq!(def.vault_id, Some(VAULT.parse().unwrap()));
        let SourceConfig::Paperless(p) = &def.config else {
            panic!("paperless")
        };
        assert_eq!(p.url, "https://paperless.example");
        assert_eq!(
            p.token_file.as_deref(),
            Some("/run/secrets/paperless_token")
        );
        assert_eq!(p.sync_mode, folio_core::models::PaperlessSyncMode::NewOnly);
        assert_eq!(p.poll_interval_secs, 600);
        assert_eq!(p.default_currency.as_deref(), Some("EUR"));
        assert_eq!(def.id, config.ingest.definitions()[0].id);
    }

    #[test]
    fn paperless_sources_load_from_environment_variables() {
        let config = load(&[
            (
                "FOLIO_INGEST__PAPERLESS__HOME__URL",
                "http://paperless:8000",
            ),
            ("FOLIO_INGEST__PAPERLESS__HOME__TOKEN", "t0ken"),
            ("FOLIO_INGEST__PAPERLESS__HOME__VAULT", VAULT),
        ]);
        let defs = config.ingest.definitions();
        assert_eq!(defs[0].vault_id, Some(VAULT.parse().unwrap()));
        assert!(!format!("{config:?}").contains("t0ken"));
    }

    #[test]
    fn paperless_source_without_a_vault_fails_naming_the_section() {
        let error = load_toml(
            r#"
[ingest.paperless.home]
url = "https://paperless.example"
token_file = "/run/secrets/paperless_token"
"#,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("ingest.paperless.home"), "{error}");
        assert!(error.contains("vault"), "{error}");
    }

    #[test]
    fn invalid_paperless_settings_fail_naming_the_section() {
        let error = load_toml(&format!(
            r#"
[ingest.paperless.home]
url = "ftp://paperless.example"
token = "t"
vault = "{VAULT}"
"#
        ))
        .unwrap_err()
        .to_string();
        assert!(error.contains("ingest.paperless.home"), "{error}");
        assert!(error.contains("url"), "{error}");
    }

    #[test]
    fn two_paperless_sources_for_one_vault_fail() {
        let error = load_toml(&format!(
            r#"
[ingest.paperless.a]
url = "https://a.example"
token = "t"
vault = "{VAULT}"

[ingest.paperless.b]
url = "https://b.example"
token = "t"
vault = "{VAULT}"
"#
        ))
        .unwrap_err()
        .to_string();
        assert!(error.contains("ingest.paperless.b"), "{error}");
    }

    #[test]
    fn malformed_ingest_variables_are_errors() {
        let result = Config::load_with(
            "/nonexistent/config.toml",
            env(&[("FOLIO_INGEST__FOLDERS__PATH", "/x")]),
        );
        assert!(result.is_err());
    }

    #[test]
    fn defaults_apply_without_file_or_env() {
        let config = load(&[]);
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.storage.path, "./data");
        assert!(config.ai.openai_api_key.is_none());
    }

    #[test]
    fn public_url_is_required_when_auth_is_exposed() {
        let exposed = |pairs: &[(&str, &str)]| load(pairs).validate();
        assert!(exposed(&[]).is_ok(), "loopback default needs no public URL");
        for host in ["127.0.0.1", "::1", "localhost"] {
            assert!(exposed(&[("FOLIO_SERVER__HOST", host)]).is_ok(), "{host}");
        }
        let err = exposed(&[("FOLIO_SERVER__HOST", "0.0.0.0")]).unwrap_err();
        assert!(err.contains("FOLIO_SERVER__PUBLIC_URL"), "{err}");
        assert!(exposed(&[("FOLIO_SERVER__HOST", "folio.lan")]).is_err());
        assert!(exposed(&[
            ("FOLIO_SERVER__HOST", "0.0.0.0"),
            ("FOLIO_SERVER__PUBLIC_URL", "https://folio.example.com"),
        ])
        .is_ok());
        assert!(exposed(&[
            ("FOLIO_SERVER__HOST", "0.0.0.0"),
            ("FOLIO_SERVER__PUBLIC_URL", "folio.example.com"),
        ])
        .is_err());
        assert!(exposed(&[
            ("FOLIO_SERVER__HOST", "0.0.0.0"),
            ("FOLIO_AUTH__REQUIRED", "false"),
        ])
        .is_ok());
    }

    #[test]
    fn imports_are_disabled_until_a_root_is_configured() {
        let config = load(&[]);
        assert!(config.import.allowed_roots.is_empty());
        assert!(!config.import.imports_enabled());
        assert_eq!(config.import.max_file_bytes, 536_870_912);
    }

    #[test]
    fn import_roots_split_on_commas_into_paths() {
        let config = load(&[("FOLIO_IMPORT__ALLOWED_ROOTS", "/imports, /mnt/paperless")]);
        assert_eq!(
            config.import.allowed_roots,
            [PathBuf::from("/imports"), PathBuf::from("/mnt/paperless")]
        );
        assert!(config.import.imports_enabled());
    }

    #[test]
    fn mcp_follows_the_auth_requirement_unless_set() {
        assert_eq!(load(&[]).mcp.enabled, None);
        assert_eq!(
            load(&[("FOLIO_MCP__ENABLED", "true")]).mcp.enabled,
            Some(true)
        );
    }

    #[test]
    fn apple_app_ids_default_to_empty() {
        assert!(load(&[]).apple.app_ids.is_empty());
    }

    #[test]
    fn apple_app_ids_load_from_a_toml_array() {
        let config = load_toml(
            "[apple]\napp_ids = [\"ABCDE12345.com.example.folio\", \"ABCDE12345.com.example.beta\"]\n",
        )
        .unwrap();
        assert_eq!(
            config.apple.app_ids,
            [
                "ABCDE12345.com.example.folio",
                "ABCDE12345.com.example.beta"
            ]
        );
        assert!(config.validate().is_ok());
    }

    #[test]
    fn apple_app_ids_env_list_overrides_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[apple]\napp_ids = [\"FGHIJ67890.org.toml\"]\n").unwrap();
        let config = Config::load_with(
            path.to_str().unwrap(),
            env(&[(
                "FOLIO_APPLE__APP_IDS",
                "ABCDE12345.com.example.folio, ABCDE12345.com.example.folio.beta",
            )]),
        )
        .unwrap();
        assert_eq!(
            config.apple.app_ids,
            [
                "ABCDE12345.com.example.folio",
                "ABCDE12345.com.example.folio.beta"
            ]
        );
    }

    #[test]
    fn invalid_apple_app_id_fails_startup_naming_the_setting() {
        let config = load(&[("FOLIO_APPLE__APP_IDS", "com.example.folio")]);
        let err = config.validate().unwrap_err();
        assert!(err.contains("apple.app_ids"), "{err}");
        assert!(err.contains("com.example.folio"), "{err}");
    }

    #[test]
    fn apple_app_links_doc_example_parses() {
        let docs = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/configuration.md"
        ))
        .unwrap();
        let section = docs
            .split("## Apple app links")
            .nth(1)
            .expect("docs/configuration.md has an Apple app links section");
        let example = section
            .split("```toml\n")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .expect("the section has a TOML example");
        let config = load_toml(example).unwrap();
        assert_eq!(config.apple.app_ids, ["ABCDE12345.com.example.folio"]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn apple_config_parses_and_dedupes_its_app_ids() {
        let config = load(&[(
            "FOLIO_APPLE__APP_IDS",
            "ABCDE12345.com.example.folio, ABCDE12345.com.example.folio",
        )]);
        let ids = config.apple.parsed().unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].as_str(), "ABCDE12345.com.example.folio");
    }

    #[test]
    fn unparseable_public_url_fails_startup_even_without_auth() {
        let err = load(&[
            ("FOLIO_AUTH__REQUIRED", "false"),
            ("FOLIO_SERVER__PUBLIC_URL", "https://exa mple.com/folio"),
        ])
        .validate()
        .unwrap_err();
        assert!(err.contains("server.public_url"), "{err}");
        assert!(load(&[
            ("FOLIO_AUTH__REQUIRED", "false"),
            ("FOLIO_SERVER__PUBLIC_URL", "https://example.com/folio"),
        ])
        .validate()
        .is_ok());
    }

    #[test]
    fn every_setting_is_set_by_its_env_var() {
        for setting in SETTINGS {
            let (sample, expected) = match (setting.key, setting.kind) {
                ("storage.backend", _) => ("s3", "s3"),
                (_, ValueKind::Str) => ("sample-value", "sample-value"),
                (_, ValueKind::Port) => ("4242", "4242"),
                (_, ValueKind::Int) => ("4242", "4242"),
                (_, ValueKind::Bool) => ("true", "true"),
                (_, ValueKind::List) => ("a.example, b.example", r#"["a.example","b.example"]"#),
            };
            let config = load(&[(setting.env, sample)]);
            assert_eq!(
                lookup(&config, setting.key).as_deref(),
                Some(expected),
                "{} did not land in {}",
                setting.env,
                setting.key
            );
        }
    }

    #[test]
    fn pipeline_environment_overrides_file_and_keeps_missing_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[ocr]\nprovider = 'none'\nlanguages = 'deu'\n[processing]\nauto_ocr = false\n",
        )
        .unwrap();
        let config = Config::load_with(
            path.to_str().unwrap(),
            env(&[
                ("FOLIO_OCR__PROVIDER", "ollama-vision"),
                ("FOLIO_EMBEDDINGS__MODEL", "nomic-embed-text"),
                ("FOLIO_PROCESSING__AUTO_EMBED", "false"),
            ]),
        )
        .unwrap();
        assert_eq!(
            lookup(&config, "ocr.provider").as_deref(),
            Some("ollama-vision")
        );
        assert_eq!(lookup(&config, "ocr.languages").as_deref(), Some("deu"));
        assert_eq!(
            lookup(&config, "embeddings.provider").as_deref(),
            Some("none")
        );
        assert_eq!(
            lookup(&config, "embeddings.model").as_deref(),
            Some("nomic-embed-text")
        );
        assert_eq!(
            lookup(&config, "processing.auto_ocr").as_deref(),
            Some("false")
        );
        assert_eq!(
            lookup(&config, "processing.auto_embed").as_deref(),
            Some("false")
        );
        assert_eq!(
            lookup(&config, "processing.auto_thumbnail").as_deref(),
            Some("true")
        );
    }

    #[test]
    fn single_underscore_aliases_and_their_file_variants_are_ignored() {
        let config = load(&[
            ("FOLIO_SERVER_HOST", "0.0.0.0"),
            ("FOLIO_SERVER_PORT", "4343"),
            ("FOLIO_DATABASE_URL", "sqlite://ignored"),
            ("FOLIO_STORAGE_PATH", "/ignored"),
            ("FOLIO_AI_PROVIDER", "ignored"),
            ("FOLIO_AI_OPENAI_API_KEY", "ignored"),
            ("FOLIO_AI_ANTHROPIC_API_KEY", "ignored"),
            ("FOLIO_SERVER_HOST_FILE", "/nonexistent/alias"),
            ("FOLIO_SERVER_PORT_FILE", "/nonexistent/alias"),
            ("FOLIO_DATABASE_URL_FILE", "/nonexistent/alias"),
            ("FOLIO_STORAGE_PATH_FILE", "/nonexistent/alias"),
            ("FOLIO_AI_PROVIDER_FILE", "/nonexistent/alias"),
            ("FOLIO_AI_OPENAI_API_KEY_FILE", "/nonexistent/alias"),
            ("FOLIO_AI_ANTHROPIC_API_KEY_FILE", "/nonexistent/alias"),
        ]);
        assert_eq!(
            serde_json::to_value(config).unwrap(),
            serde_json::to_value(Config::default()).unwrap()
        );
    }

    #[test]
    fn forward_auth_lists_split_on_commas_and_spaces() {
        let config = load(&[
            ("FOLIO_AUTH__FORWARD__ENABLED", "true"),
            (
                "FOLIO_AUTH__FORWARD__TRUSTED_PROXIES",
                "10.0.0.0/8, 172.16.0.5, fd00::/8",
            ),
            ("FOLIO_AUTH__FORWARD__ADMIN_GROUP", "admins"),
        ]);
        assert!(config.auth.forward.enabled);
        assert_eq!(
            config.auth.forward.trusted_proxies,
            ["10.0.0.0/8", "172.16.0.5", "fd00::/8"]
        );
        assert_eq!(config.auth.forward.admin_group.as_deref(), Some("admins"));
        assert_eq!(config.auth.forward.user_header, "Remote-User");
        assert!(
            config.auth.forward.groups_header.is_empty()
                && config.auth.forward.role_header.is_empty(),
            "group and role headers must be opt-in"
        );
    }

    #[test]
    fn oidc_defaults_and_secret_file() {
        let config = load(&[]);
        let oidc = &config.auth.oidc;
        assert!(!oidc.enabled);
        assert_eq!(oidc.scopes, "openid profile email");
        assert_eq!(oidc.groups_claim, "groups");
        assert!(oidc.allow_local_login && oidc.auto_create_users);

        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("secret");
        std::fs::write(&secret, "s3cret\n").unwrap();
        let config = load(&[
            ("FOLIO_AUTH__OIDC__ENABLED", "true"),
            (
                "FOLIO_AUTH__OIDC__ISSUER",
                "https://auth.example.com/application/o/folio/",
            ),
            (
                "FOLIO_AUTH__OIDC__CLIENT_SECRET_FILE",
                secret.to_str().unwrap(),
            ),
        ]);
        assert!(config.auth.oidc.enabled);
        assert_eq!(config.auth.oidc.client_secret.as_deref(), Some("s3cret"));
        assert!(!format!("{:?}", config.auth.oidc).contains("s3cret"));
    }

    #[test]
    fn env_beats_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[server]\nhost = \"10.0.0.1\"\nport = 8080\n").unwrap();
        let config = Config::load_with(
            path.to_str().unwrap(),
            env(&[("FOLIO_SERVER__PORT", "9090")]),
        )
        .unwrap();
        assert_eq!(config.server.host, "10.0.0.1");
        assert_eq!(config.server.port, 9090);
    }

    #[test]
    fn file_suffix_reads_secret_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join("openai");
        std::fs::write(&secret, "sk-from-file\n").unwrap();
        let config = load(&[("FOLIO_AI__OPENAI_API_KEY_FILE", secret.to_str().unwrap())]);
        assert_eq!(config.ai.openai_api_key.as_deref(), Some("sk-from-file"));
    }

    #[test]
    fn value_and_file_together_is_an_error() {
        let result = Config::load_with(
            "/nonexistent/config.toml",
            env(&[
                ("FOLIO_AI__OPENAI_API_KEY", "a"),
                ("FOLIO_AI__OPENAI_API_KEY_FILE", "/dev/null"),
            ]),
        );
        assert!(result.is_err());
    }

    #[test]
    fn missing_secret_file_is_an_error() {
        let result = Config::load_with(
            "/nonexistent/config.toml",
            env(&[("FOLIO_AI__OPENAI_API_KEY_FILE", "/nonexistent/secret")]),
        );
        assert!(result.is_err());
    }

    #[test]
    fn invalid_port_is_an_error() {
        let result = Config::load_with(
            "/nonexistent/config.toml",
            env(&[("FOLIO_SERVER__PORT", "http")]),
        );
        assert!(result.is_err());
    }

    #[test]
    fn registry_covers_every_config_key() {
        fn leaves(prefix: &str, value: &serde_json::Value, out: &mut Vec<String>) {
            match value {
                serde_json::Value::Object(map) => {
                    for (k, v) in map {
                        let key = if prefix.is_empty() {
                            k.clone()
                        } else {
                            format!("{prefix}.{k}")
                        };
                        leaves(&key, v, out);
                    }
                }
                _ => out.push(prefix.to_string()),
            }
        }
        let mut keys = Vec::new();
        leaves(
            "",
            &serde_json::to_value(Config::default()).unwrap(),
            &mut keys,
        );
        for key in keys {
            assert!(
                SETTINGS.iter().any(|s| s.key == key),
                "{key} is missing from SETTINGS"
            );
        }
    }

    #[test]
    fn docs_list_every_env_var() {
        let docs = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/configuration.md"
        ))
        .unwrap();
        for setting in SETTINGS {
            assert!(
                docs.contains(setting.env),
                "{} is not documented in docs/configuration.md",
                setting.env
            );
        }
    }
}
