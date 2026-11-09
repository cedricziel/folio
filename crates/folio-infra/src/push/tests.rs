use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use folio_core::models::Setting;
use folio_core::Result;

use super::apns::TEST_KEY;
use super::*;
use crate::config::{ApnsConfig, PushConfig};

#[derive(Default)]
struct Settings(Mutex<HashMap<String, String>>);

impl Settings {
    fn with(rows: &[(&str, &str)]) -> Arc<Self> {
        Arc::new(Self(Mutex::new(
            rows.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )))
    }
}

fn row(key: &str, value: &str) -> Setting {
    Setting {
        key: key.into(),
        value: value.into(),
        description: None,
        updated_at: Utc::now(),
    }
}

#[async_trait]
impl SettingsRepository for Settings {
    async fn get(&self, key: &str) -> Result<Option<Setting>> {
        Ok(self.0.lock().unwrap().get(key).map(|v| row(key, v)))
    }
    async fn list(&self) -> Result<Vec<Setting>> {
        self.list_by_prefix("").await
    }
    async fn list_by_prefix(&self, prefix: &str) -> Result<Vec<Setting>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| row(k, v))
            .collect())
    }
    async fn set(&self, key: &str, value: &str) -> Result<Setting> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(row(key, value))
    }
    async fn set_many(&self, settings: &[(String, String)]) -> Result<()> {
        for (k, v) in settings {
            self.set(k, v).await?;
        }
        Ok(())
    }
    async fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

fn configured() -> PushConfig {
    PushConfig {
        transport: "apns".into(),
        apns: ApnsConfig {
            team_id: Some("TEAM".into()),
            key_id: Some("KEY".into()),
            private_key: Some(TEST_KEY.into()),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn secret() -> Arc<SecretBox> {
    Arc::new(SecretBox::new("a-secret-key-long-enough-for-tests").unwrap())
}

#[tokio::test]
async fn unconfigured_push_is_off() {
    let push = ConfiguredPush::new(Settings::with(&[]), PushConfig::default(), None, false);

    assert!(!push.available().await);
    let status = push.status().await;
    assert_eq!(status.transport, "none");
    assert_eq!(status.transport_source, Source::Default);
    assert_eq!(status.problem, None);
    assert_eq!(push.limits().await, PushLimits::default());
}

#[tokio::test]
async fn configuration_alone_enables_apns() {
    let push = ConfiguredPush::new(Settings::with(&[]), configured(), None, false);

    assert!(push.available().await);
    let status = push.status().await;
    assert_eq!(status.transport_source, Source::Config);
    assert_eq!(status.key_source, Source::Config);
}

#[tokio::test]
async fn database_rows_win() {
    let settings = Settings::with(&[(QUIET_WINDOW, "5"), (MAX_ALERTS, "6"), (TRANSPORT, "none")]);
    let push = ConfiguredPush::new(settings, configured(), None, false);

    assert!(!push.available().await);
    let limits = push.limits().await;
    assert_eq!(limits.quiet_window, chrono::Duration::seconds(5));
    assert_eq!(limits.max_alerts_per_hour, 6);
}

#[tokio::test]
async fn a_sealed_key_is_read_back_and_an_empty_one_falls_back() {
    let secret = secret();
    let settings = Settings::with(&[(PRIVATE_KEY, &seal_key(&secret, TEST_KEY))]);
    let mut config = configured();
    config.apns.private_key = None;
    let push = ConfiguredPush::new(settings.clone(), config, Some(secret.clone()), true);
    assert!(push.available().await);
    assert_eq!(push.status().await.key_source, Source::Database);
    assert!(push.can_store_key());

    settings.set(PRIVATE_KEY, "").await.unwrap();
    push.invalidate().await;
    assert!(
        !push.available().await,
        "no key in the database or configuration"
    );
}

#[tokio::test]
async fn a_key_sealed_with_another_secret_is_unusable() {
    let other = SecretBox::new("another-secret-long-enough-for-tests").unwrap();
    let settings = Settings::with(&[(PRIVATE_KEY, &seal_key(&other, TEST_KEY))]);
    let mut config = configured();
    config.apns.private_key = None;
    let push = ConfiguredPush::new(settings, config, Some(secret()), true);

    assert!(!push.available().await);
}

#[tokio::test]
async fn keys_may_only_be_stored_with_an_external_secret() {
    let push = ConfiguredPush::new(Settings::with(&[]), configured(), Some(secret()), false);
    assert!(!push.can_store_key());
}

#[test]
fn sealed_keys_do_not_contain_the_key() {
    let sealed = seal_key(&secret(), TEST_KEY);
    assert!(sealed.starts_with("enc:v1:"));
    assert!(!sealed.contains("BEGIN PRIVATE KEY"));
    assert_eq!(
        open_key(Some(&secret()), &sealed).as_deref(),
        Some(TEST_KEY)
    );
    assert_eq!(open_key(None, &sealed), None);
    assert_eq!(open_key(None, "plain").as_deref(), Some("plain"));
}

#[tokio::test]
async fn a_credential_error_pauses_sending() {
    let push = ConfiguredPush::new(Settings::with(&[]), configured(), None, false);
    assert!(push.available().await);
    {
        let mut health = push.health.lock().unwrap();
        health.paused_until = Some(Instant::now() + Duration::from_secs(60));
    }
    assert!(!push.available().await);
    push.invalidate().await;
    assert!(push.available().await, "a settings change lifts the pause");
}
