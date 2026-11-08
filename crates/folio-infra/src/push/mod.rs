//! Push notification delivery: effective settings and the APNs transport
//!
//! Settings come from `config.toml` and the `settings` table, where the table
//! wins. The `.p8` key is stored encrypted, and only when the encryption
//! secret does not live in the same database.

pub mod apns;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use tracing::warn;

use folio_auth::secret::SecretBox;
use folio_core::ports::{
    PushLimits, PushLimitsSource, PushMessage, PushOutcome, PushTransport, SettingsRepository,
};

use crate::config::PushConfig;
use apns::{ApnsClient, ApnsCredentials};

pub const TRANSPORT: &str = "push.transport";
pub const TEAM_ID: &str = "push.apns.team_id";
pub const KEY_ID: &str = "push.apns.key_id";
pub const TOPIC: &str = "push.apns.topic";
pub const PRIVATE_KEY: &str = "push.apns.private_key";
pub const QUIET_WINDOW: &str = "push.quiet_window_seconds";
pub const MAX_ALERTS: &str = "push.max_alerts_per_hour";

const ENCRYPTED_PREFIX: &str = "enc:v1:";
const KEY_CONTEXT: &str = "push.apns.private_key";
const CACHE_FOR: Duration = Duration::from_secs(5);
const PAUSE_AFTER_CREDENTIAL_ERROR: Duration = Duration::from_secs(15 * 60);

/// Where an effective value came from
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Default,
    Config,
    Database,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Config => "configuration",
            Self::Database => "database",
        }
    }
}

/// The push settings in effect
#[derive(Clone)]
pub struct EffectivePush {
    pub transport: (String, Source),
    pub team_id: (Option<String>, Source),
    pub key_id: (Option<String>, Source),
    pub topic: (String, Source),
    pub private_key: (Option<String>, Source),
    pub base_url: Option<String>,
    pub limits: PushLimits,
}

impl EffectivePush {
    fn credentials(&self) -> Option<ApnsCredentials> {
        if self.transport.0 != "apns" {
            return None;
        }
        Some(ApnsCredentials {
            team_id: self.team_id.0.clone()?,
            key_id: self.key_id.0.clone()?,
            topic: self.topic.0.clone(),
            private_key: self.private_key.0.clone()?,
            base_url: self.base_url.clone(),
        })
    }
}

/// The last time APNs refused the credentials
#[derive(Debug, Clone)]
pub struct CredentialError {
    pub reason: String,
    pub at: DateTime<Utc>,
}

/// What the settings page shows
#[derive(Debug, Clone)]
pub struct PushStatus {
    pub transport: String,
    pub transport_source: Source,
    pub team_id: Option<String>,
    pub key_id: Option<String>,
    pub topic: String,
    pub key_present: bool,
    pub key_source: Source,
    pub base_url: Option<String>,
    pub quiet_window_seconds: i64,
    pub max_alerts_per_hour: u32,
    pub available: bool,
    pub problem: Option<String>,
    pub last_error: Option<CredentialError>,
    /// Whether a key may be saved in the database
    pub can_store_key: bool,
}

/// Seal a `.p8` key for the settings table
pub fn seal_key(secret_box: &SecretBox, pem: &str) -> String {
    format!(
        "{ENCRYPTED_PREFIX}{}",
        secret_box.seal(pem.as_bytes(), KEY_CONTEXT)
    )
}

/// Read a stored key: sealed values need the box, plain ones pass through
pub fn open_key(secret_box: Option<&SecretBox>, stored: &str) -> Option<String> {
    match stored.strip_prefix(ENCRYPTED_PREFIX) {
        Some(sealed) => secret_box?
            .open(sealed, KEY_CONTEXT)
            .and_then(|bytes| String::from_utf8(bytes).ok()),
        None => Some(stored.to_string()),
    }
}

struct Health {
    error: Option<CredentialError>,
    paused_until: Option<Instant>,
}

/// Push delivery as configured right now
pub struct ConfiguredPush {
    settings: Arc<dyn SettingsRepository>,
    config: PushConfig,
    secret_box: Option<Arc<SecretBox>>,
    /// The encryption secret comes from configuration, not the database
    external_secret: bool,
    cache: RwLock<Option<(Instant, Arc<EffectivePush>)>>,
    client: Mutex<Option<(ApnsCredentials, Arc<ApnsClient>)>>,
    health: Mutex<Health>,
}

impl ConfiguredPush {
    pub fn new(
        settings: Arc<dyn SettingsRepository>,
        config: PushConfig,
        secret_box: Option<Arc<SecretBox>>,
        external_secret: bool,
    ) -> Self {
        if let Some(base) = &config.apns.base_url {
            warn!("push.apns.base_url is set: pushes go to {base}, not to Apple");
        }
        Self {
            settings,
            config,
            secret_box,
            external_secret,
            cache: RwLock::new(None),
            client: Mutex::new(None),
            health: Mutex::new(Health {
                error: None,
                paused_until: None,
            }),
        }
    }

    /// Whether a `.p8` key may be stored in the database
    pub fn can_store_key(&self) -> bool {
        self.external_secret && self.secret_box.is_some()
    }

    pub fn secret_box(&self) -> Option<&SecretBox> {
        self.secret_box.as_deref()
    }

    /// Drop cached settings, so a save applies at once
    pub async fn invalidate(&self) {
        *self.cache.write().await = None;
        let mut health = self.health.lock().unwrap();
        health.paused_until = None;
    }

    pub async fn effective(&self) -> Arc<EffectivePush> {
        if let Some((at, effective)) = self.cache.read().await.as_ref() {
            if at.elapsed() < CACHE_FOR {
                return effective.clone();
            }
        }
        let effective = Arc::new(self.load().await);
        *self.cache.write().await = Some((Instant::now(), effective.clone()));
        effective
    }

    async fn load(&self) -> EffectivePush {
        let rows = match self.settings.list_by_prefix("push.").await {
            Ok(rows) => rows,
            Err(e) => {
                warn!("Could not read push settings: {e}");
                Vec::new()
            }
        };
        let db = |key: &str| {
            rows.iter()
                .find(|row| row.key == key)
                .map(|row| row.value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let pick = |key: &str, configured: Option<String>, default: Option<String>| match (
            db(key),
            configured.filter(|v| !v.is_empty()),
        ) {
            (Some(value), _) => (Some(value), Source::Database),
            (None, Some(value)) => (Some(value), Source::Config),
            (None, None) => (default, Source::Default),
        };
        let config = &self.config;
        let (transport, transport_source) = pick(
            TRANSPORT,
            Some(config.transport.clone()).filter(|t| t != "none"),
            Some("none".into()),
        );
        let (topic, topic_source) = pick(
            TOPIC,
            Some(config.apns.topic.clone()).filter(|t| t != "com.cedricziel.folio"),
            Some("com.cedricziel.folio".into()),
        );
        let private_key = match db(PRIVATE_KEY) {
            Some(stored) => {
                match open_key(self.secret_box.as_deref(), &stored) {
                    Some(key) => (Some(key), Source::Database),
                    None => {
                        warn!("The stored APNs key cannot be decrypted with the current auth.secret_key");
                        (None, Source::Database)
                    }
                }
            }
            None => match config.apns.private_key.clone().filter(|k| !k.is_empty()) {
                Some(key) => (Some(key), Source::Config),
                None => (None, Source::Default),
            },
        };
        let number = |key: &str, configured: u64| {
            db(key)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(configured)
        };
        EffectivePush {
            transport: (transport.unwrap_or_else(|| "none".into()), transport_source),
            team_id: pick(TEAM_ID, config.apns.team_id.clone(), None),
            key_id: pick(KEY_ID, config.apns.key_id.clone(), None),
            topic: (topic.unwrap_or_default(), topic_source),
            private_key,
            base_url: config.apns.base_url.clone().filter(|u| !u.is_empty()),
            limits: PushLimits {
                quiet_window: chrono::Duration::seconds(
                    number(QUIET_WINDOW, config.quiet_window_seconds).min(3600) as i64,
                ),
                max_alerts_per_hour: number(MAX_ALERTS, config.max_alerts_per_hour).clamp(1, 3600)
                    as u32,
            },
        }
    }

    /// The client for the effective credentials, or why there is none
    async fn client(&self) -> Result<Arc<ApnsClient>, String> {
        let effective = self.effective().await;
        let credentials = match effective.credentials() {
            Some(credentials) => credentials,
            None if effective.transport.0 == "none" => return Err("push is off".into()),
            None if effective.transport.0 != "apns" => {
                return Err(format!("unknown transport '{}'", effective.transport.0))
            }
            None => return Err("APNs needs a team ID, a key ID and a private key".into()),
        };
        let mut slot = self.client.lock().unwrap();
        if let Some((current, client)) = slot.as_ref() {
            if *current == credentials {
                return Ok(client.clone());
            }
        }
        let client = Arc::new(ApnsClient::new(credentials.clone())?);
        *slot = Some((credentials, client.clone()));
        // New credentials get a fresh chance.
        let mut health = self.health.lock().unwrap();
        health.paused_until = None;
        Ok(client)
    }

    pub async fn status(&self) -> PushStatus {
        let effective = self.effective().await;
        let problem = self.client().await.err();
        let available = self.available().await;
        let health = self.health.lock().unwrap();
        PushStatus {
            transport: effective.transport.0.clone(),
            transport_source: effective.transport.1,
            team_id: effective.team_id.0.clone(),
            key_id: effective.key_id.0.clone(),
            topic: effective.topic.0.clone(),
            key_present: effective.private_key.0.is_some(),
            key_source: effective.private_key.1,
            base_url: effective.base_url.clone(),
            quiet_window_seconds: effective.limits.quiet_window.num_seconds(),
            max_alerts_per_hour: effective.limits.max_alerts_per_hour,
            available,
            problem: problem.filter(|p| p != "push is off"),
            last_error: health.error.clone(),
            can_store_key: self.can_store_key(),
        }
    }
}

#[async_trait]
impl PushTransport for ConfiguredPush {
    async fn available(&self) -> bool {
        if self.client().await.is_err() {
            return false;
        }
        let health = self.health.lock().unwrap();
        health
            .paused_until
            .is_none_or(|until| Instant::now() >= until)
    }

    async fn send(&self, message: &PushMessage) -> PushOutcome {
        let client = match self.client().await {
            Ok(client) => client,
            Err(reason) => return PushOutcome::Retry { reason },
        };
        let outcome = client.send(message).await;
        if let PushOutcome::CredentialError { reason } = &outcome {
            warn!(%reason, "APNs refused the provider token; pausing push notifications");
            let mut health = self.health.lock().unwrap();
            health.error = Some(CredentialError {
                reason: reason.clone(),
                at: Utc::now(),
            });
            health.paused_until = Some(Instant::now() + PAUSE_AFTER_CREDENTIAL_ERROR);
        }
        outcome
    }
}

#[async_trait]
impl PushLimitsSource for ConfiguredPush {
    async fn limits(&self) -> PushLimits {
        self.effective().await.limits
    }
}

#[cfg(test)]
mod tests;
