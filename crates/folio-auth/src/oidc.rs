//! OpenID Connect relying party: discovery, code exchange and ID token
//! validation against the provider's rotating JWKS.

use std::time::{Duration, Instant};

use jsonwebtoken::{decode, decode_header, jwk::Jwk, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::crypto::ct_eq;

const METADATA_TTL: Duration = Duration::from_secs(3600);
const JWKS_TTL: Duration = Duration::from_secs(3600);
/// Minimum gap between refetches triggered by an unknown `kid`, so a token
/// with a bogus key id cannot make Folio hammer the provider.
const JWKS_MIN_REFRESH: Duration = Duration::from_secs(30);
/// Asymmetric algorithms only; `none` and HMAC are never accepted.
const ALLOWED_ALGS: [Algorithm; 9] = [
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::EdDSA,
];

#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub scopes: String,
    pub display_name: String,
    pub allow_local_login: bool,
    pub admin_group: Option<String>,
    pub groups_claim: String,
    pub auto_create_users: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("provider request failed: {0}")]
    Http(String),
    #[error("provider metadata is invalid: {0}")]
    Metadata(String),
    #[error("token endpoint refused the code: {0}")]
    Token(String),
    #[error("invalid ID token: {0}")]
    IdToken(String),
}

#[derive(Debug, Clone, Deserialize)]
struct Metadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    end_session_endpoint: Option<String>,
}

/// The verified identity claims Folio uses.
#[derive(Debug, Clone)]
pub struct IdClaims {
    pub subject: String,
    pub preferred_username: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
    pub groups: Vec<String>,
}

struct Jwks {
    keys: Vec<Jwk>,
    fetched: Instant,
}

pub struct OidcClient {
    pub config: OidcConfig,
    http: reqwest::Client,
    metadata: RwLock<Option<(Metadata, Instant)>>,
    jwks: RwLock<Option<Jwks>>,
    jwks_min_refresh: Duration,
}

impl OidcClient {
    pub fn new(config: OidcConfig) -> Result<Self, String> {
        let issuer = url::Url::parse(&config.issuer).map_err(|e| format!("invalid issuer: {e}"))?;
        if !matches!(issuer.scheme(), "https" | "http") {
            return Err("issuer must be an http(s) URL".into());
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            config,
            http,
            metadata: RwLock::new(None),
            jwks: RwLock::new(None),
            jwks_min_refresh: JWKS_MIN_REFRESH,
        })
    }

    /// Overrides the minimum gap between key refetches (tests).
    pub fn with_jwks_min_refresh(mut self, gap: Duration) -> Self {
        self.jwks_min_refresh = gap;
        self
    }

    async fn metadata(&self) -> Result<Metadata, OidcError> {
        if let Some((m, at)) = self.metadata.read().await.as_ref() {
            if at.elapsed() < METADATA_TTL {
                return Ok(m.clone());
            }
        }
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.config.issuer.trim_end_matches('/')
        );
        let meta: Metadata = self.get_json(&url).await?;
        if meta.issuer.trim_end_matches('/') != self.config.issuer.trim_end_matches('/') {
            return Err(OidcError::Metadata(format!(
                "issuer mismatch: expected {}, got {}",
                self.config.issuer, meta.issuer
            )));
        }
        *self.metadata.write().await = Some((meta.clone(), Instant::now()));
        Ok(meta)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T, OidcError> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| OidcError::Http(e.to_string()))?;
        if !response.status().is_success() {
            return Err(OidcError::Http(format!(
                "{url} returned {}",
                response.status()
            )));
        }
        response
            .json()
            .await
            .map_err(|e| OidcError::Http(e.to_string()))
    }

    pub async fn authorization_url(
        &self,
        redirect_uri: &str,
        state: &str,
        nonce: &str,
        code_challenge: &str,
    ) -> Result<String, OidcError> {
        let meta = self.metadata().await?;
        let mut url = url::Url::parse(&meta.authorization_endpoint)
            .map_err(|e| OidcError::Metadata(e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", &self.config.scopes)
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(url.into())
    }

    /// RP-initiated logout URL, when the provider advertises one.
    pub async fn end_session_url(&self, post_logout_redirect_uri: &str) -> Option<String> {
        let endpoint = self.metadata().await.ok()?.end_session_endpoint?;
        let mut url = url::Url::parse(&endpoint).ok()?;
        url.query_pairs_mut()
            .append_pair("client_id", &self.config.client_id)
            .append_pair("post_logout_redirect_uri", post_logout_redirect_uri);
        Some(url.into())
    }

    /// Exchanges the authorization code and returns the validated identity.
    pub async fn exchange(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
        expected_nonce: &str,
    ) -> Result<IdClaims, OidcError> {
        let meta = self.metadata().await?;
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ];
        let mut request = self.http.post(&meta.token_endpoint);
        match &self.config.client_secret {
            Some(secret) => request = request.basic_auth(&self.config.client_id, Some(secret)),
            None => form.push(("client_id", &self.config.client_id)),
        }
        let response = request
            .form(&form)
            .send()
            .await
            .map_err(|e| OidcError::Http(e.to_string()))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|e| OidcError::Token(format!("unreadable response ({status}): {e}")))?;
        if !status.is_success() {
            return Err(OidcError::Token(format!("{status}: {body}")));
        }
        let id_token = body
            .get("id_token")
            .and_then(Value::as_str)
            .ok_or_else(|| OidcError::Token("no id_token in the response".into()))?;
        self.verify_id_token(&meta, id_token, expected_nonce).await
    }

    async fn verify_id_token(
        &self,
        meta: &Metadata,
        token: &str,
        expected_nonce: &str,
    ) -> Result<IdClaims, OidcError> {
        let bad = |m: &str| OidcError::IdToken(m.to_string());
        let header = decode_header(token).map_err(|e| OidcError::IdToken(e.to_string()))?;
        if !ALLOWED_ALGS.contains(&header.alg) {
            return Err(bad("unsupported signature algorithm"));
        }
        let key = self.decoding_key(meta, header.kid.as_deref()).await?;

        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[meta.issuer.as_str()]);
        validation.set_audience(&[self.config.client_id.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        let data = decode::<Value>(token, &key, &validation)
            .map_err(|e| OidcError::IdToken(e.to_string()))?;
        let claims = data.claims;

        match claims.get("nonce").and_then(Value::as_str) {
            Some(n) if ct_eq(n, expected_nonce) => {}
            _ => return Err(bad("nonce mismatch")),
        }
        if claims
            .get("aud")
            .and_then(Value::as_array)
            .is_some_and(|a| a.len() > 1)
            && claims.get("azp").and_then(Value::as_str) != Some(self.config.client_id.as_str())
        {
            return Err(bad("azp does not match the client"));
        }
        let text = |name: &str| {
            claims
                .get(name)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let groups = match claims.get(&self.config.groups_claim) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            Some(Value::String(s)) => s.split(',').map(|g| g.trim().to_string()).collect(),
            _ => Vec::new(),
        };
        Ok(IdClaims {
            subject: text("sub").ok_or_else(|| bad("missing sub"))?,
            preferred_username: text("preferred_username"),
            email: text("email"),
            name: text("name"),
            groups,
        })
    }

    async fn decoding_key(
        &self,
        meta: &Metadata,
        kid: Option<&str>,
    ) -> Result<DecodingKey, OidcError> {
        if let Some(key) = self.cached_key(kid, false).await? {
            return Ok(key);
        }
        self.refresh_jwks(meta).await?;
        self.cached_key(kid, true)
            .await?
            .ok_or_else(|| OidcError::IdToken("no matching signing key".into()))
    }

    /// Looks the key up in the cache. A stale cache counts as a miss; so does
    /// an unknown `kid` unless the cache was just refreshed (`fresh`).
    async fn cached_key(
        &self,
        kid: Option<&str>,
        fresh: bool,
    ) -> Result<Option<DecodingKey>, OidcError> {
        let guard = self.jwks.read().await;
        let Some(jwks) = guard.as_ref() else {
            return Ok(None);
        };
        if !fresh && jwks.fetched.elapsed() >= JWKS_TTL {
            return Ok(None);
        }
        let found = match kid {
            Some(kid) => jwks
                .keys
                .iter()
                .find(|k| k.common.key_id.as_deref() == Some(kid)),
            None if jwks.keys.len() == 1 => jwks.keys.first(),
            None => None,
        };
        found
            .map(|jwk| DecodingKey::from_jwk(jwk).map_err(|e| OidcError::IdToken(e.to_string())))
            .transpose()
    }

    async fn refresh_jwks(&self, meta: &Metadata) -> Result<(), OidcError> {
        if let Some(jwks) = self.jwks.read().await.as_ref() {
            if jwks.fetched.elapsed() < self.jwks_min_refresh {
                return Ok(());
            }
        }
        let raw: Value = self.get_json(&meta.jwks_uri).await?;
        let keys = raw
            .get("keys")
            .and_then(Value::as_array)
            .ok_or_else(|| OidcError::Metadata("JWKS has no keys".into()))?
            .iter()
            .filter(|k| k.get("use").and_then(Value::as_str) != Some("enc"))
            .filter_map(|k| serde_json::from_value::<Jwk>(k.clone()).ok())
            .collect();
        *self.jwks.write().await = Some(Jwks {
            keys,
            fetched: Instant::now(),
        });
        Ok(())
    }
}
