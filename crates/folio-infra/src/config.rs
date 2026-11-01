//! Configuration management

use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::Deserialize;

/// Application configuration
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub storage: StorageConfig,
    pub ai: AiConfig,
    #[serde(default)]
    pub mcp: McpConfig,
}

/// MCP server configuration
#[derive(Debug, Clone, Deserialize)]
pub struct McpConfig {
    /// Serve the MCP endpoint at `/mcp`; the `mcp.enabled` DB setting overrides this
    #[serde(default = "default_mcp_enabled")]
    pub enabled: bool,
    /// Extra `Host` header values to accept (besides loopback), needed when
    /// reaching Folio under another hostname
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

fn default_mcp_enabled() -> bool {
    true
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: default_mcp_enabled(),
            allowed_hosts: Vec::new(),
        }
    }
}

/// Server configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3000
}

/// Database configuration
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_database_url")]
    pub url: String,
}

fn default_database_url() -> String {
    "sqlite:folio.db?mode=rwc".to_string()
}

/// Storage configuration
#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_storage_path")]
    pub path: String,
}

fn default_storage_path() -> String {
    "./data".to_string()
}

/// AI provider configuration
#[derive(Debug, Clone, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_ai_provider")]
    pub provider: String,

    #[serde(default)]
    pub openai_api_key: Option<String>,

    #[serde(default)]
    pub anthropic_api_key: Option<String>,
}

fn default_ai_provider() -> String {
    "openai".to_string()
}

impl Config {
    /// Load configuration from file and environment
    #[allow(clippy::result_large_err)]
    pub fn load() -> Result<Self, figment::Error> {
        // Load .env file if present (for development)
        let _ = dotenvy::dotenv();

        Figment::new()
            .merge(Toml::file("config.toml"))
            .merge(Env::prefixed("FOLIO_").split("_"))
            .extract()
    }

    /// Load configuration from a specific file path
    #[allow(clippy::result_large_err)]
    pub fn load_from(path: &str) -> Result<Self, figment::Error> {
        let _ = dotenvy::dotenv();

        Figment::new()
            .merge(Toml::file(path))
            .merge(Env::prefixed("FOLIO_").split("_"))
            .extract()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: default_host(),
                port: default_port(),
            },
            database: DatabaseConfig {
                url: default_database_url(),
            },
            storage: StorageConfig {
                path: default_storage_path(),
            },
            ai: AiConfig {
                provider: default_ai_provider(),
                openai_api_key: None,
                anthropic_api_key: None,
            },
            mcp: McpConfig::default(),
        }
    }
}
