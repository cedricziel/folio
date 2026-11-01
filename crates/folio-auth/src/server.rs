//! OAuth 2.1 authorization server logic (no HTTP types beyond JSON).

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    crypto::{ct_eq, hash_token, random_token, verify_pkce},
    model::{AccessToken, AuthCode, OAuthClient, RefreshToken, SCOPES},
    redirect::{redirect_uri_matches, validate_redirect_uri},
    store::{AuthError, AuthStore},
};

pub const CODE_TTL_SECS: i64 = 60;
pub const ACCESS_TTL_SECS: i64 = 3600;
pub const REFRESH_TTL_SECS: i64 = 30 * 24 * 3600;
/// Paths of protected resources a token may be bound to.
pub const RESOURCE_PATHS: [&str; 1] = ["/mcp"];

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// An RFC 6749 error response.
#[derive(Debug)]
pub struct OAuthError {
    pub error: &'static str,
    pub description: String,
}

impl OAuthError {
    fn new(error: &'static str, description: impl Into<String>) -> Self {
        Self {
            error,
            description: description.into(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "error": self.error, "error_description": self.description })
    }
}

impl From<AuthError> for OAuthError {
    fn from(e: AuthError) -> Self {
        tracing::error!("auth store failure: {e}");
        Self::new("server_error", "internal error")
    }
}

#[derive(Debug, Deserialize)]
pub struct RegistrationRequest {
    pub client_name: Option<String>,
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: Option<String>,
    pub grant_types: Option<Vec<String>>,
    pub response_types: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AuthorizeParams {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub state: Option<String>,
    pub scope: Option<String>,
    pub resource: Option<String>,
}

/// An authorization request that passed validation.
#[derive(Debug, Clone)]
pub struct ValidatedAuthorize {
    pub client: OAuthClient,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub state: Option<String>,
    pub scopes: Vec<String>,
    pub audience: String,
}

#[derive(Debug)]
pub enum AuthorizeFailure {
    /// The client or redirect URI cannot be trusted: show an error page.
    Page(String),
    /// Report the error to the client via its redirect URI.
    Redirect {
        redirect_uri: String,
        error: &'static str,
        description: String,
        state: Option<String>,
    },
}

#[derive(Debug, Default, Deserialize)]
pub struct TokenRequest {
    pub grant_type: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub client_id: Option<String>,
    pub refresh_token: Option<String>,
    pub resource: Option<String>,
    pub scope: Option<String>,
}

fn trim_slash(s: &str) -> &str {
    s.trim_end_matches('/')
}

pub fn authorization_server_metadata(base: &str) -> Value {
    json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "revocation_endpoint": format!("{base}/oauth/revoke"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": SCOPES,
    })
}

pub fn protected_resource_metadata(base: &str) -> Value {
    json!({
        "resource": format!("{base}{}", RESOURCE_PATHS[0]),
        "authorization_servers": [base],
        "scopes_supported": SCOPES,
        "bearer_methods_supported": ["header"],
    })
}

fn parse_scopes(scope: Option<&str>) -> Result<Vec<String>, String> {
    match scope.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(SCOPES.iter().map(|s| s.to_string()).collect()),
        Some(s) => {
            let mut out: Vec<String> = Vec::new();
            for item in s.split_whitespace() {
                if !SCOPES.contains(&item) {
                    return Err(format!("unknown scope: {item}"));
                }
                if !out.iter().any(|o| o == item) {
                    out.push(item.to_string());
                }
            }
            Ok(out)
        }
    }
}

fn resolve_audience(base: &str, resource: Option<&str>) -> Result<String, String> {
    let known = |path: &str| format!("{base}{path}");
    match resource.filter(|r| !r.is_empty()) {
        None => Ok(known(RESOURCE_PATHS[0])),
        Some(r) => RESOURCE_PATHS
            .iter()
            .map(|p| known(p))
            .find(|k| trim_slash(k) == trim_slash(r))
            .ok_or_else(|| format!("unknown resource: {r}")),
    }
}

/// A `resource` sent at the token endpoint must match the granted audience.
fn check_resource(base: &str, resource: Option<&str>, granted: &str) -> Result<(), OAuthError> {
    if resource.is_none() {
        return Ok(());
    }
    let audience =
        resolve_audience(base, resource).map_err(|m| OAuthError::new("invalid_target", m))?;
    if audience != granted {
        return Err(OAuthError::new(
            "invalid_target",
            "resource differs from the original grant",
        ));
    }
    Ok(())
}

#[derive(Clone)]
pub struct OAuthServer {
    store: Arc<dyn AuthStore>,
}

impl OAuthServer {
    pub fn new(store: Arc<dyn AuthStore>) -> Self {
        Self { store }
    }

    /// RFC 7591 registration of a public client.
    pub async fn register_client(&self, req: RegistrationRequest) -> Result<Value, OAuthError> {
        let invalid = |m: &str| OAuthError::new("invalid_client_metadata", m);
        if req.redirect_uris.is_empty() || req.redirect_uris.len() > 10 {
            return Err(OAuthError::new(
                "invalid_redirect_uri",
                "provide between 1 and 10 redirect_uris",
            ));
        }
        for uri in &req.redirect_uris {
            validate_redirect_uri(uri).map_err(|m| OAuthError::new("invalid_redirect_uri", m))?;
        }
        if req
            .token_endpoint_auth_method
            .as_deref()
            .is_some_and(|m| m != "none")
        {
            return Err(invalid(
                "only token_endpoint_auth_method \"none\" is supported",
            ));
        }
        let allowed_grants = ["authorization_code", "refresh_token"];
        if let Some(grants) = &req.grant_types {
            if grants.iter().any(|g| !allowed_grants.contains(&g.as_str())) {
                return Err(invalid("unsupported grant_types"));
            }
        }
        if let Some(types) = &req.response_types {
            if types.iter().any(|t| t != "code") {
                return Err(invalid("unsupported response_types"));
            }
        }
        let name: String = req
            .client_name
            .map(|n| n.trim().chars().take(100).collect())
            .filter(|n: &String| !n.is_empty())
            .unwrap_or_else(|| "Unnamed application".to_string());

        let client = OAuthClient {
            client_id: uuid::Uuid::new_v4().to_string(),
            client_name: name,
            redirect_uris: req.redirect_uris,
            created_at: now(),
        };
        self.store.insert_client(client.clone()).await?;
        Ok(json!({
            "client_id": client.client_id,
            "client_id_issued_at": client.created_at,
            "client_name": client.client_name,
            "redirect_uris": client.redirect_uris,
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "scope": SCOPES.join(" "),
        }))
    }

    pub async fn validate_authorize(
        &self,
        base: &str,
        p: &AuthorizeParams,
    ) -> Result<ValidatedAuthorize, AuthorizeFailure> {
        let page = |m: &str| AuthorizeFailure::Page(m.to_string());
        let client_id = p
            .client_id
            .as_deref()
            .ok_or_else(|| page("Missing client_id."))?;
        let client = self
            .store
            .find_client(client_id)
            .await
            .map_err(|e| {
                tracing::error!("client lookup failed: {e}");
                page("Something went wrong.")
            })?
            .ok_or_else(|| page("This application is not registered."))?;
        let redirect_uri = p
            .redirect_uri
            .clone()
            .ok_or_else(|| page("Missing redirect_uri."))?;
        if !client
            .redirect_uris
            .iter()
            .any(|r| redirect_uri_matches(r, &redirect_uri))
        {
            return Err(page(
                "The redirect address does not match the registration.",
            ));
        }

        let fail = |error, description: &str| AuthorizeFailure::Redirect {
            redirect_uri: redirect_uri.clone(),
            error,
            description: description.to_string(),
            state: p.state.clone(),
        };
        if p.response_type.as_deref() != Some("code") {
            return Err(fail(
                "unsupported_response_type",
                "response_type must be code",
            ));
        }
        let challenge = p
            .code_challenge
            .clone()
            .filter(|c| !c.is_empty())
            .ok_or_else(|| fail("invalid_request", "code_challenge is required"))?;
        if p.code_challenge_method.as_deref() != Some("S256") {
            return Err(fail(
                "invalid_request",
                "code_challenge_method must be S256",
            ));
        }
        let scopes = parse_scopes(p.scope.as_deref()).map_err(|m| fail("invalid_scope", &m))?;
        let audience = resolve_audience(base, p.resource.as_deref())
            .map_err(|m| fail("invalid_target", &m))?;

        Ok(ValidatedAuthorize {
            client,
            redirect_uri: redirect_uri.clone(),
            code_challenge: challenge,
            state: p.state.clone(),
            scopes,
            audience,
        })
    }

    /// Issues a single-use authorization code valid for 60 seconds.
    pub async fn issue_code(
        &self,
        user_id: &str,
        v: &ValidatedAuthorize,
    ) -> Result<String, AuthError> {
        let code = random_token();
        self.store
            .insert_code(AuthCode {
                code_hash: hash_token(&code),
                client_id: v.client.client_id.clone(),
                user_id: user_id.to_string(),
                redirect_uri: v.redirect_uri.clone(),
                code_challenge: v.code_challenge.clone(),
                scopes: v.scopes.clone(),
                audience: v.audience.clone(),
                expires_at: now() + CODE_TTL_SECS,
            })
            .await?;
        Ok(code)
    }

    pub async fn token(&self, base: &str, req: TokenRequest) -> Result<Value, OAuthError> {
        let _ = self.store.purge_expired(now()).await;
        let client_id = req
            .client_id
            .as_deref()
            .ok_or_else(|| OAuthError::new("invalid_client", "client_id is required"))?;
        self.store
            .find_client(client_id)
            .await?
            .ok_or_else(|| OAuthError::new("invalid_client", "unknown client"))?;

        match req.grant_type.as_deref() {
            Some("authorization_code") => self.code_grant(base, client_id, &req).await,
            Some("refresh_token") => self.refresh_grant(base, client_id, &req).await,
            _ => Err(OAuthError::new(
                "unsupported_grant_type",
                "use authorization_code or refresh_token",
            )),
        }
    }

    async fn code_grant(
        &self,
        base: &str,
        client_id: &str,
        req: &TokenRequest,
    ) -> Result<Value, OAuthError> {
        let grant_err = |m: &str| OAuthError::new("invalid_grant", m);
        let code = req
            .code
            .as_deref()
            .ok_or_else(|| OAuthError::new("invalid_request", "code is required"))?;
        let verifier = req
            .code_verifier
            .as_deref()
            .ok_or_else(|| OAuthError::new("invalid_request", "code_verifier is required"))?;
        // Consumed before any check so a failed attempt burns the code.
        let record = self
            .store
            .take_code(&hash_token(code))
            .await?
            .ok_or_else(|| grant_err("invalid or already used code"))?;
        if record.expires_at <= now() {
            return Err(grant_err("code expired"));
        }
        if !ct_eq(&record.client_id, client_id) {
            return Err(grant_err("code was issued to another client"));
        }
        if req.redirect_uri.as_deref() != Some(record.redirect_uri.as_str()) {
            return Err(grant_err("redirect_uri does not match"));
        }
        if !verify_pkce(verifier, &record.code_challenge) {
            return Err(grant_err("PKCE verification failed"));
        }
        check_resource(base, req.resource.as_deref(), &record.audience)?;
        self.issue_tokens(client_id, &record.user_id, record.scopes, record.audience)
            .await
    }

    async fn refresh_grant(
        &self,
        base: &str,
        client_id: &str,
        req: &TokenRequest,
    ) -> Result<Value, OAuthError> {
        let grant_err = |m: &str| OAuthError::new("invalid_grant", m);
        let token = req
            .refresh_token
            .as_deref()
            .ok_or_else(|| OAuthError::new("invalid_request", "refresh_token is required"))?;
        let record = self
            .store
            .take_refresh_token(&hash_token(token))
            .await?
            .ok_or_else(|| grant_err("invalid refresh token"))?;
        if record.expires_at <= now() || !ct_eq(&record.client_id, client_id) {
            return Err(grant_err("invalid refresh token"));
        }
        check_resource(base, req.resource.as_deref(), &record.audience)?;
        let scopes = match req.scope.as_deref() {
            None => record.scopes.clone(),
            Some(s) => {
                let requested =
                    parse_scopes(Some(s)).map_err(|m| OAuthError::new("invalid_scope", m))?;
                if requested.iter().any(|r| !record.scopes.contains(r)) {
                    return Err(OAuthError::new("invalid_scope", "cannot widen scope"));
                }
                requested
            }
        };
        self.issue_tokens(client_id, &record.user_id, scopes, record.audience)
            .await
    }

    async fn issue_tokens(
        &self,
        client_id: &str,
        user_id: &str,
        scopes: Vec<String>,
        audience: String,
    ) -> Result<Value, OAuthError> {
        let access = random_token();
        let refresh = random_token();
        let issued = now();
        self.store
            .insert_access_token(AccessToken {
                token_hash: hash_token(&access),
                client_id: client_id.to_string(),
                user_id: user_id.to_string(),
                scopes: scopes.clone(),
                audience: audience.clone(),
                expires_at: issued + ACCESS_TTL_SECS,
            })
            .await?;
        self.store
            .insert_refresh_token(RefreshToken {
                token_hash: hash_token(&refresh),
                client_id: client_id.to_string(),
                user_id: user_id.to_string(),
                scopes: scopes.clone(),
                audience,
                expires_at: issued + REFRESH_TTL_SECS,
            })
            .await?;
        Ok(json!({
            "access_token": access,
            "token_type": "Bearer",
            "expires_in": ACCESS_TTL_SECS,
            "refresh_token": refresh,
            "scope": scopes.join(" "),
        }))
    }

    /// RFC 7009: unknown tokens are not an error.
    pub async fn revoke(&self, token: &str) -> Result<(), AuthError> {
        self.store.revoke_token(&hash_token(token)).await
    }
}
