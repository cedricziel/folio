//! OpenID Connect relying party: sign in through an external identity provider
//! with the authorization code flow, PKCE (S256), state and nonce.
//!
//! [`OidcProvider`] owns discovery (cached, refreshed hourly), ID token
//! validation (signature through the provider's JWKS, issuer, audience, expiry
//! and nonce) and the mapping of claims to an [`ExternalIdentity`]. Storing the
//! in-flight login and creating the session is left to the HTTP layer.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use openidconnect::{
    core::{CoreAuthenticationFlow, CoreClient, CoreJsonWebKeySet},
    reqwest::Client as HttpClient,
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use openidconnect::{
    ClaimsVerificationError, ProviderMetadataWithLogout, SignatureVerificationError,
};
use serde_json::Value;
use tokio::sync::{Mutex, RwLock};
use url::Url;

use crate::{external::ExternalIdentity, model::SOURCE_OIDC_PREFIX};

const DISCOVERY_TTL: Duration = Duration::from_secs(3600);
/// Minimum time between JWKS refetches triggered by an unknown `kid`, so a
/// forged token cannot make Folio hammer the provider.
const KID_MISS_COOLDOWN: Duration = Duration::from_secs(10);
const CLOCK_LEEWAY_SECS: i64 = 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct OidcSettings {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub scopes: Vec<String>,
    pub display_name: String,
    pub groups_claim: String,
    pub admin_group: Option<String>,
    pub allow_local_login: bool,
    pub auto_create_users: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    /// The provider could not be reached or answered with something unusable.
    #[error("identity provider error: {0}")]
    Provider(String),
    /// The provider answered, but the login must not be accepted.
    #[error("login rejected: {0}")]
    Rejected(String),
}

/// An authorization request ready to send the browser to.
#[derive(Debug, Clone)]
pub struct AuthRequest {
    pub url: Url,
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

struct Loaded {
    metadata: ProviderMetadataWithLogout,
    fetched: Instant,
}

pub struct OidcProvider {
    settings: OidcSettings,
    source: String,
    http: HttpClient,
    loaded: RwLock<Option<Arc<Loaded>>>,
    last_kid_refresh: Mutex<Option<Instant>>,
}

impl OidcProvider {
    pub fn new(settings: OidcSettings) -> Result<Self, String> {
        let issuer = Url::parse(&settings.issuer).map_err(|e| format!("invalid issuer: {e}"))?;
        let loopback = match issuer.host() {
            Some(url::Host::Domain(d)) => d == "localhost",
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        };
        if issuer.scheme() != "https" && !(issuer.scheme() == "http" && loopback) {
            return Err("the issuer must be an https URL".into());
        }
        if settings.client_id.trim().is_empty() {
            return Err("client_id is required".into());
        }
        let http = HttpClient::builder()
            // Redirects could point discovery at internal services.
            .redirect(openidconnect::reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Self {
            source: format!(
                "{SOURCE_OIDC_PREFIX}{}",
                settings.issuer.trim_end_matches('/')
            ),
            settings,
            http,
            loaded: RwLock::new(None),
            last_kid_refresh: Mutex::new(None),
        })
    }

    /// The `auth_source` stored on users who sign in through this provider.
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn display_name(&self) -> &str {
        &self.settings.display_name
    }

    pub fn allow_local_login(&self) -> bool {
        self.settings.allow_local_login
    }

    pub fn auto_create_users(&self) -> bool {
        self.settings.auto_create_users
    }

    pub fn admin_group(&self) -> Option<&str> {
        self.settings.admin_group.as_deref()
    }

    /// Fetches the discovery document ahead of the first login.
    pub async fn warm_up(&self) -> Result<(), OidcError> {
        self.metadata().await.map(|_| ())
    }

    async fn fetch_metadata(&self) -> Result<Loaded, OidcError> {
        let issuer = IssuerUrl::new(self.settings.issuer.clone())
            .map_err(|e| OidcError::Provider(format!("invalid issuer: {e}")))?;
        let metadata = ProviderMetadataWithLogout::discover_async(issuer, &self.http)
            .await
            .map_err(|e| OidcError::Provider(format!("discovery failed: {e}")))?;
        Ok(Loaded {
            metadata,
            fetched: Instant::now(),
        })
    }

    async fn metadata(&self) -> Result<Arc<Loaded>, OidcError> {
        if let Some(l) = self.loaded.read().await.as_ref() {
            if l.fetched.elapsed() < DISCOVERY_TTL {
                return Ok(l.clone());
            }
        }
        let mut slot = self.loaded.write().await;
        if let Some(l) = slot.as_ref() {
            if l.fetched.elapsed() < DISCOVERY_TTL {
                return Ok(l.clone());
            }
        }
        match self.fetch_metadata().await {
            Ok(fresh) => {
                let fresh = Arc::new(fresh);
                *slot = Some(fresh.clone());
                Ok(fresh)
            }
            Err(e) => match slot.as_ref() {
                Some(stale) => {
                    tracing::warn!("{e}; using the cached discovery document");
                    Ok(stale.clone())
                }
                None => Err(e),
            },
        }
    }

    /// Refetches the JWKS after a token named a key we do not know. Returns
    /// whether new keys are now available to try.
    async fn refresh_jwks(&self, seen: &Arc<Loaded>) -> Result<bool, OidcError> {
        let mut last = self.last_kid_refresh.lock().await;
        if last.is_some_and(|t| t.elapsed() < KID_MISS_COOLDOWN) {
            return Ok(false);
        }
        *last = Some(Instant::now());
        let jwks = CoreJsonWebKeySet::fetch_async(seen.metadata.jwks_uri(), &self.http)
            .await
            .map_err(|e| OidcError::Provider(format!("fetching the JWKS failed: {e}")))?;
        let mut slot = self.loaded.write().await;
        let current = slot.as_ref().unwrap_or(seen);
        *slot = Some(Arc::new(Loaded {
            metadata: current.metadata.clone().set_jwks(jwks),
            fetched: current.fetched,
        }));
        Ok(true)
    }

    fn client(
        &self,
        loaded: &Loaded,
        redirect_uri: &str,
    ) -> Result<
        CoreClient<
            openidconnect::EndpointSet,
            openidconnect::EndpointNotSet,
            openidconnect::EndpointNotSet,
            openidconnect::EndpointNotSet,
            openidconnect::EndpointMaybeSet,
            openidconnect::EndpointMaybeSet,
        >,
        OidcError,
    > {
        let redirect = RedirectUrl::new(redirect_uri.to_string())
            .map_err(|e| OidcError::Provider(format!("invalid redirect URI: {e}")))?;
        Ok(CoreClient::from_provider_metadata(
            loaded.metadata.clone(),
            ClientId::new(self.settings.client_id.clone()),
            self.settings.client_secret.clone().map(ClientSecret::new),
        )
        .set_redirect_uri(redirect))
    }

    /// Starts a login: returns the provider URL plus the secrets the callback
    /// needs to finish it. The caller must store them bound to the browser.
    pub async fn authorize(&self, redirect_uri: &str) -> Result<AuthRequest, OidcError> {
        let loaded = self.metadata().await?;
        let client = self.client(&loaded, redirect_uri)?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let mut request = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge);
        for scope in &self.settings.scopes {
            request = request.add_scope(Scope::new(scope.clone()));
        }
        let (url, state, nonce) = request.url();
        Ok(AuthRequest {
            url,
            state: state.secret().clone(),
            nonce: nonce.secret().clone(),
            pkce_verifier: verifier.secret().clone(),
        })
    }

    /// Finishes a login: exchanges the code, validates the ID token and maps
    /// the claims to an identity.
    pub async fn complete(
        &self,
        redirect_uri: &str,
        code: &str,
        nonce: &str,
        pkce_verifier: &str,
    ) -> Result<ExternalIdentity, OidcError> {
        let loaded = self.metadata().await?;
        let client = self.client(&loaded, redirect_uri)?;
        let tokens = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .map_err(|e| OidcError::Provider(format!("no token endpoint: {e}")))?
            .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_string()))
            .request_async(&self.http)
            .await
            .map_err(|e| OidcError::Rejected(format!("token exchange failed: {e}")))?;
        let id_token = tokens
            .id_token()
            .ok_or_else(|| OidcError::Rejected("the provider returned no ID token".into()))?;
        let nonce = Nonce::new(nonce.to_string());

        let verify = |loaded: &Loaded| -> Result<String, ClaimsVerificationError> {
            let client = self
                .client(loaded, redirect_uri)
                .map_err(|e| ClaimsVerificationError::Other(e.to_string()))?;
            let verifier = client
                .id_token_verifier()
                .set_time_fn(|| chrono::Utc::now() - chrono::Duration::seconds(CLOCK_LEEWAY_SECS))
                .set_issue_time_verifier_fn(|iat| {
                    let latest = chrono::Utc::now() + chrono::Duration::seconds(CLOCK_LEEWAY_SECS);
                    if iat > latest {
                        Err(format!("ID token issued in the future ({iat})"))
                    } else {
                        Ok(())
                    }
                });
            let claims = id_token.claims(&verifier, &nonce)?;
            Ok(claims.subject().to_string())
        };
        let subject = match verify(&loaded) {
            Ok(sub) => sub,
            Err(ClaimsVerificationError::SignatureVerification(
                SignatureVerificationError::NoMatchingKey,
            )) => {
                if !self.refresh_jwks(&loaded).await? {
                    return Err(OidcError::Rejected(
                        "the ID token is signed with an unknown key".into(),
                    ));
                }
                let refreshed = self.metadata().await?;
                verify(&refreshed)
                    .map_err(|e| OidcError::Rejected(format!("invalid ID token: {e}")))?
            }
            Err(e) => return Err(OidcError::Rejected(format!("invalid ID token: {e}"))),
        };

        let mut claims = id_token_payload(id_token)?;
        let wanted = |claims: &Value| {
            claim_str(claims, "preferred_username").is_some()
                && claims.get(&self.settings.groups_claim).is_some()
        };
        if !wanted(&claims) {
            if let Some(endpoint) = loaded.metadata.userinfo_endpoint() {
                match self
                    .userinfo(endpoint.url(), tokens.access_token().secret(), &subject)
                    .await
                {
                    Ok(Value::Object(info)) => {
                        if let Value::Object(map) = &mut claims {
                            for (k, v) in info {
                                map.entry(k).or_insert(v);
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("userinfo request failed: {e}"),
                }
            }
        }
        self.identity(&subject, &claims)
    }

    async fn userinfo(&self, endpoint: &Url, token: &str, subject: &str) -> Result<Value, String> {
        let response = self
            .http
            .get(endpoint.as_str())
            .bearer_auth(token)
            .header("accept", "application/json")
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?;
        let body = response.bytes().await.map_err(|e| e.to_string())?;
        let info: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
        // A mismatching subject means the response belongs to someone else.
        if info.get("sub").and_then(Value::as_str) != Some(subject) {
            return Err("userinfo subject does not match the ID token".into());
        }
        Ok(info)
    }

    fn identity(&self, subject: &str, claims: &Value) -> Result<ExternalIdentity, OidcError> {
        let email = claim_str(claims, "email");
        let username = claim_str(claims, "preferred_username")
            .or_else(|| email.clone())
            .unwrap_or_else(|| subject.to_string());
        let groups = match claims.get(&self.settings.groups_claim) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            Some(Value::String(s)) => s
                .split([',', ' '])
                .filter(|g| !g.is_empty())
                .map(str::to_string)
                .collect(),
            _ => Vec::new(),
        };
        Ok(ExternalIdentity {
            source: self.source.clone(),
            external_id: subject.to_string(),
            username,
            email,
            display_name: claim_str(claims, "name"),
            groups,
        })
    }

    /// The provider's RP-initiated logout URL, when it advertises one.
    pub async fn logout_url(&self, post_logout_redirect: &str) -> Option<Url> {
        let loaded = self.metadata().await.ok()?;
        let endpoint = loaded
            .metadata
            .additional_metadata()
            .end_session_endpoint
            .clone()?;
        let mut url = endpoint.url().clone();
        url.query_pairs_mut()
            .append_pair("client_id", &self.settings.client_id)
            .append_pair("post_logout_redirect_uri", post_logout_redirect);
        Some(url)
    }
}

fn claim_str(claims: &Value, name: &str) -> Option<String> {
    claims
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The claims of an ID token whose signature has already been verified, kept
/// as JSON so provider-specific claims such as groups stay reachable.
fn id_token_payload(id_token: &impl serde::Serialize) -> Result<Value, OidcError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let jwt = serde_json::to_value(id_token)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .ok_or_else(|| OidcError::Rejected("unreadable ID token".into()))?;
    let payload = jwt
        .split('.')
        .nth(1)
        .and_then(|p| URL_SAFE_NO_PAD.decode(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| OidcError::Rejected("unreadable ID token".into()))?;
    Ok(payload)
}
