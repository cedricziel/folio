//! Token-based APNs over HTTP/2
//!
//! The provider token is an ES256 JWT signed with the team's `.p8` key. Apple
//! wants it refreshed no more often than every 20 minutes and at least every
//! 60, so it is re-signed after 50.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use p256::pkcs8::DecodePrivateKey;
use serde_json::json;

use folio_core::models::PushEnvironment;
use folio_core::ports::{PushMessage, PushOutcome, PushType};

const PRODUCTION: &str = "https://api.push.apple.com";
const SANDBOX: &str = "https://api.sandbox.push.apple.com";
const TOKEN_LIFETIME: Duration = Duration::from_secs(50 * 60);

/// Parse a `.p8` key: PKCS#8 PEM holding a P-256 private key
pub fn parse_key(pem: &str) -> Result<SigningKey, String> {
    SigningKey::from_pkcs8_pem(pem.trim())
        .map_err(|_| "not a PKCS#8 P-256 private key (.p8)".to_string())
}

/// What the client needs to reach APNs
#[derive(Clone, PartialEq, Eq)]
pub struct ApnsCredentials {
    pub team_id: String,
    pub key_id: String,
    pub topic: String,
    pub private_key: String,
    /// Replaces both Apple hosts; `http://` means cleartext HTTP/2
    pub base_url: Option<String>,
}

impl std::fmt::Debug for ApnsCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApnsCredentials")
            .field("team_id", &self.team_id)
            .field("key_id", &self.key_id)
            .field("topic", &self.topic)
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

pub struct ApnsClient {
    http: reqwest::Client,
    credentials: ApnsCredentials,
    key: SigningKey,
    token: Mutex<Option<(String, Instant)>>,
}

impl ApnsClient {
    pub fn new(credentials: ApnsCredentials) -> Result<Self, String> {
        let key = parse_key(&credentials.private_key)?;
        if credentials.team_id.is_empty() || credentials.key_id.is_empty() {
            return Err("team ID and key ID are required".into());
        }
        let http = reqwest::Client::builder()
            .http2_prior_knowledge()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            credentials,
            key,
            token: Mutex::new(None),
        })
    }

    /// The cached provider token, signed again after 50 minutes or when `renew` is set
    fn provider_token(&self, renew: bool) -> String {
        let mut cached = self.token.lock().unwrap();
        if let Some((token, at)) = cached.as_ref() {
            if !renew && at.elapsed() < TOKEN_LIFETIME {
                return token.clone();
            }
        }
        let token = sign(
            &self.key,
            &self.credentials.key_id,
            &self.credentials.team_id,
            Utc::now().timestamp(),
        );
        *cached = Some((token.clone(), Instant::now()));
        token
    }

    fn url(&self, message: &PushMessage) -> String {
        let base = match (&self.credentials.base_url, message.environment) {
            (Some(base), _) => base.trim_end_matches('/'),
            (None, PushEnvironment::Production) => PRODUCTION,
            (None, PushEnvironment::Sandbox) => SANDBOX,
        };
        format!("{base}/3/device/{}", message.token)
    }

    pub async fn send(&self, message: &PushMessage) -> PushOutcome {
        let outcome = self.attempt(message, false).await;
        match &outcome {
            PushOutcome::CredentialError { reason } if reason == "ExpiredProviderToken" => {
                self.attempt(message, true).await
            }
            _ => outcome,
        }
    }

    async fn attempt(&self, message: &PushMessage, renew: bool) -> PushOutcome {
        let mut request = self
            .http
            .post(self.url(message))
            .bearer_auth(self.provider_token(renew))
            .header("apns-id", message.apns_id.to_string())
            .header("apns-push-type", push_type(message.push_type))
            .header(
                "apns-topic",
                topic(&self.credentials.topic, message.push_type),
            )
            .header("apns-priority", message.priority.to_string())
            .header(
                "apns-expiration",
                message.expiration.timestamp().to_string(),
            )
            .json(&message.payload);
        if let Some(collapse_id) = &message.collapse_id {
            request = request.header("apns-collapse-id", collapse_id);
        }
        match request.send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                let body = response.text().await.unwrap_or_default();
                outcome(status, &body)
            }
            Err(e) => PushOutcome::Retry {
                reason: format!("network: {e}"),
            },
        }
    }
}

fn push_type(push_type: PushType) -> &'static str {
    match push_type {
        PushType::Alert => "alert",
        PushType::Widgets => "widgets",
    }
}

fn topic(bundle: &str, push_type: PushType) -> String {
    match push_type {
        PushType::Alert => bundle.to_string(),
        PushType::Widgets => format!("{bundle}.push-type.widgets"),
    }
}

/// An ES256 provider token
pub fn sign(key: &SigningKey, key_id: &str, team_id: &str, issued_at: i64) -> String {
    let header = URL_SAFE_NO_PAD.encode(json!({ "alg": "ES256", "kid": key_id }).to_string());
    let claims = URL_SAFE_NO_PAD.encode(json!({ "iss": team_id, "iat": issued_at }).to_string());
    let input = format!("{header}.{claims}");
    let signature: Signature = key.sign(input.as_bytes());
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

/// Map an APNs response to what the sender should do
pub fn outcome(status: u16, body: &str) -> PushOutcome {
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let reason = parsed["reason"].as_str().unwrap_or_default().to_string();
    match (status, reason.as_str()) {
        (200, _) => PushOutcome::Delivered,
        (410, _) => PushOutcome::TokenInvalid {
            since: parsed["timestamp"]
                .as_i64()
                .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
                .map(|at: DateTime<Utc>| at),
        },
        (400, "BadDeviceToken" | "DeviceTokenNotForTopic") => {
            PushOutcome::TokenInvalid { since: None }
        }
        (403, _) => PushOutcome::CredentialError {
            reason: if reason.is_empty() {
                "Forbidden".into()
            } else {
                reason
            },
        },
        (429, _) | (500..=599, _) => PushOutcome::Retry {
            reason: if reason.is_empty() {
                format!("HTTP {status}")
            } else {
                reason
            },
        },
        _ => PushOutcome::Rejected {
            reason: if reason.is_empty() {
                format!("HTTP {status}")
            } else {
                reason
            },
        },
    }
}

/// A throwaway P-256 key for tests
#[cfg(test)]
pub(crate) const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgAosoCcYoVHPZlCWP
oShkVBX+C6hjdfOCLb99IVGjenWhRANCAAQHz9RTjiH1cvtgtneWb5gLPuVYpIpN
KAuAQloidgaveNCZtHQFTnDTR4pFwYBKElRLGQnBgDU8trF2AC/LsCzk
-----END PRIVATE KEY-----
";

#[cfg(test)]
mod tests;
