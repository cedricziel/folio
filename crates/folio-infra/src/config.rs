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
    pub auth: AuthConfig,
}

/// Server configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Externally visible base URL (for example `https://folio.example.com`).
    /// Used in OAuth metadata and redirects; derived from the request when unset.
    #[serde(default)]
    pub public_url: Option<String>,
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

/// Authentication configuration
#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// Require a session or token for the web UI and API.
    #[serde(default = "default_auth_required")]
    pub required: bool,
}

fn default_auth_required() -> bool {
    true
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            required: default_auth_required(),
        }
    }
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

/// Maps `FOLIO_SERVER_PUBLIC_URL` to `server.public_url`: only the first
/// underscore separates section from key, since keys contain underscores.
fn env_provider() -> Env {
    Env::prefixed("FOLIO_").map(|key| key.as_str().replacen('_', ".", 1).into())
}

impl Config {
    /// Load configuration from file and environment
    #[allow(clippy::result_large_err)]
    pub fn load() -> Result<Self, figment::Error> {
        // Load .env file if present (for development)
        let _ = dotenvy::dotenv();

        Figment::new()
            .merge(Toml::file("config.toml"))
            .merge(env_provider())
            .extract()
    }

    /// Load configuration from a specific file path
    #[allow(clippy::result_large_err)]
    pub fn load_from(path: &str) -> Result<Self, figment::Error> {
        let _ = dotenvy::dotenv();

        Figment::new()
            .merge(Toml::file(path))
            .merge(env_provider())
            .extract()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: default_host(),
                port: default_port(),
                public_url: None,
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
            auth: AuthConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::result_large_err)]
    fn env_maps_to_nested_keys_with_underscores() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("FOLIO_SERVER_PUBLIC_URL", "https://folio.example.com");
            jail.set_env("FOLIO_SERVER_PORT", "4000");
            jail.set_env("FOLIO_DATABASE_URL", "sqlite::memory:");
            jail.set_env("FOLIO_STORAGE_PATH", "./data");
            jail.set_env("FOLIO_AI_OPENAI_API_KEY", "sk-test");
            jail.set_env("FOLIO_AUTH_REQUIRED", "false");
            let config = Config::load_from("missing.toml")?;
            assert_eq!(
                config.server.public_url.as_deref(),
                Some("https://folio.example.com")
            );
            assert_eq!(config.server.port, 4000);
            assert!(!config.auth.required);
            assert_eq!(config.ai.openai_api_key.as_deref(), Some("sk-test"));
            Ok(())
        });
    }
}
