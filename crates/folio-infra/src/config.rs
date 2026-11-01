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

use figment::{
    providers::{Format, Serialized, Toml},
    Figment,
};
use folio_core::models::{
    EmailConfig, FolderConfig, Id, IngestionSourceDef, SourceConfig, SourceOrigin,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

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
    pub ingest: IngestConfig,
    pub auth: AuthConfig,
    pub mcp: McpConfig,
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
/// ```
///
/// The same keys work as environment variables, for example
/// `FOLIO_INGEST__FOLDERS__INBOX__PATH=/consume`. See `docs/ingestion.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IngestConfig {
    pub folders: BTreeMap<String, Configured<FolderConfig>>,
    pub email: BTreeMap<String, Configured<EmailConfig>>,
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
            id: config_source_id("folder", name),
            name: name.clone(),
            enabled: c.enabled,
            origin: SourceOrigin::Config,
            config: SourceConfig::Folder(c.settings.clone()),
        });
        let mailboxes = self.email.iter().map(|(name, c)| IngestionSourceDef {
            id: config_source_id("email", name),
            name: name.clone(),
            enabled: c.enabled,
            origin: SourceOrigin::Config,
            config: SourceConfig::Email(c.settings.clone()),
        });
        folders.chain(mailboxes).collect()
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
        match &self.public_url {
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

/// Database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    pub url: String,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// Authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    /// Require a session or token for the web UI and API.
    pub required: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self { required: true }
    }
}

/// AI provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub provider: String,
    pub openai_api_key: Option<String>,
    pub anthropic_api_key: Option<String>,
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

/// How an environment value is converted before it reaches the config tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Str,
    Port,
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
    /// Deprecated environment variables that still work.
    pub legacy_env: &'static [&'static str],
    pub kind: ValueKind,
}

pub const SETTINGS: &[Setting] = &[
    Setting {
        key: "server.host",
        env: "FOLIO_SERVER__HOST",
        legacy_env: &["FOLIO_SERVER_HOST"],
        kind: ValueKind::Str,
    },
    Setting {
        key: "server.port",
        env: "FOLIO_SERVER__PORT",
        legacy_env: &["FOLIO_SERVER_PORT"],
        kind: ValueKind::Port,
    },
    Setting {
        key: "server.public_url",
        env: "FOLIO_SERVER__PUBLIC_URL",
        legacy_env: &[],
        kind: ValueKind::Str,
    },
    Setting {
        key: "auth.required",
        env: "FOLIO_AUTH__REQUIRED",
        legacy_env: &[],
        kind: ValueKind::Bool,
    },
    Setting {
        key: "database.url",
        env: "FOLIO_DATABASE__URL",
        legacy_env: &["FOLIO_DATABASE_URL"],
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.backend",
        env: "FOLIO_STORAGE__BACKEND",
        legacy_env: &[],
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.path",
        env: "FOLIO_STORAGE__PATH",
        legacy_env: &["FOLIO_STORAGE_PATH"],
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.bucket",
        env: "FOLIO_STORAGE__S3__BUCKET",
        legacy_env: &[],
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.region",
        env: "FOLIO_STORAGE__S3__REGION",
        legacy_env: &[],
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.endpoint",
        env: "FOLIO_STORAGE__S3__ENDPOINT",
        legacy_env: &[],
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.access_key_id",
        env: "FOLIO_STORAGE__S3__ACCESS_KEY_ID",
        legacy_env: &[],
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.secret_access_key",
        env: "FOLIO_STORAGE__S3__SECRET_ACCESS_KEY",
        legacy_env: &[],
        kind: ValueKind::Str,
    },
    Setting {
        key: "storage.s3.path_style",
        env: "FOLIO_STORAGE__S3__PATH_STYLE",
        legacy_env: &[],
        kind: ValueKind::Bool,
    },
    Setting {
        key: "storage.s3.allow_http",
        env: "FOLIO_STORAGE__S3__ALLOW_HTTP",
        legacy_env: &[],
        kind: ValueKind::Bool,
    },
    Setting {
        key: "storage.s3.prefix",
        env: "FOLIO_STORAGE__S3__PREFIX",
        legacy_env: &[],
        kind: ValueKind::Str,
    },
    Setting {
        key: "ai.provider",
        env: "FOLIO_AI__PROVIDER",
        legacy_env: &["FOLIO_AI_PROVIDER"],
        kind: ValueKind::Str,
    },
    Setting {
        key: "ai.openai_api_key",
        env: "FOLIO_AI__OPENAI_API_KEY",
        legacy_env: &["FOLIO_AI_OPENAI_API_KEY"],
        kind: ValueKind::Str,
    },
    Setting {
        key: "ai.anthropic_api_key",
        env: "FOLIO_AI__ANTHROPIC_API_KEY",
        legacy_env: &["FOLIO_AI_ANTHROPIC_API_KEY"],
        kind: ValueKind::Str,
    },
    Setting {
        key: "mcp.enabled",
        env: "FOLIO_MCP__ENABLED",
        legacy_env: &[],
        kind: ValueKind::Bool,
    },
    Setting {
        key: "mcp.allowed_hosts",
        env: "FOLIO_MCP__ALLOWED_HOSTS",
        legacy_env: &[],
        kind: ValueKind::List,
    },
];

impl Config {
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
            if let Some(value) = resolve_env(setting, &vars)? {
                figment =
                    figment.merge(Serialized::default(setting.key, to_value(setting, value)?));
            }
        }
        if let Some(ingest) = ingest_from_env(&vars)? {
            figment = figment.merge(Serialized::default("ingest", ingest));
        }
        figment.extract()
    }
}

const INGEST_ENV_PREFIX: &str = "FOLIO_INGEST__";

/// Collect `FOLIO_INGEST__<FOLDERS|EMAIL>__<NAME>__<KEY>` variables into a tree
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
                "{base}: expected FOLIO_INGEST__<FOLDERS|EMAIL>__<NAME>__<KEY>"
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

/// Find the value for a setting: canonical name first, then deprecated
/// aliases, each also accepted with a `_FILE` suffix.
#[allow(clippy::result_large_err)]
fn resolve_env(
    setting: &Setting,
    vars: &HashMap<String, String>,
) -> Result<Option<String>, figment::Error> {
    let names = std::iter::once(setting.env).chain(setting.legacy_env.iter().copied());
    for name in names {
        let Some(value) = read_var(name, vars)? else {
            continue;
        };
        if name != setting.env {
            warn!(
                "{name} is deprecated and will be removed in a future release; use {} instead",
                setting.env
            );
        }
        return Ok(Some(value));
    }
    Ok(None)
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
    fn mcp_follows_the_auth_requirement_unless_set() {
        assert_eq!(load(&[]).mcp.enabled, None);
        assert_eq!(
            load(&[("FOLIO_MCP__ENABLED", "true")]).mcp.enabled,
            Some(true)
        );
    }

    #[test]
    fn every_setting_is_set_by_its_env_var() {
        for setting in SETTINGS {
            let (sample, expected) = match (setting.key, setting.kind) {
                ("storage.backend", _) => ("s3", "s3"),
                (_, ValueKind::Str) => ("sample-value", "sample-value"),
                (_, ValueKind::Port) => ("4242", "4242"),
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
    fn legacy_names_still_work() {
        for setting in SETTINGS {
            for legacy in setting.legacy_env {
                let (sample, expected) = match setting.kind {
                    ValueKind::Str => ("legacy-value", "legacy-value"),
                    ValueKind::Port => ("4343", "4343"),
                    ValueKind::Bool => ("true", "true"),
                    ValueKind::List => ("legacy.example", r#"["legacy.example"]"#),
                };
                let config = load(&[(legacy, sample)]);
                assert_eq!(lookup(&config, setting.key).as_deref(), Some(expected));
            }
        }
    }

    #[test]
    fn canonical_name_beats_legacy_alias() {
        let config = load(&[
            ("FOLIO_AI_OPENAI_API_KEY", "old"),
            ("FOLIO_AI__OPENAI_API_KEY", "new"),
        ]);
        assert_eq!(config.ai.openai_api_key.as_deref(), Some("new"));
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
