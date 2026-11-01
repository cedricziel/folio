//! Bearer token validation for resource servers (MCP, later REST).

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{
    crypto::{ct_eq, hash_token},
    model::AuthContext,
    server::now,
    store::AuthStore,
};

/// Public base URL without trailing slash. Uses the configured value, or
/// derives it from the request (`X-Forwarded-Proto` + `Host`) for local dev.
pub fn base_url(configured: Option<&str>, headers: &HeaderMap) -> String {
    if let Some(url) = configured.filter(|u| !u.is_empty()) {
        return url.trim_end_matches('/').to_string();
    }
    let header_str = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let scheme = header_str("x-forwarded-proto").unwrap_or("http");
    let host = header_str("host").unwrap_or("localhost");
    format!("{scheme}://{host}")
}

/// Validates opaque access tokens issued by Folio's authorization server.
///
/// `resource_path` is the protected path (for example `/mcp`); tokens must
/// have been issued for `<public base URL><resource_path>` (RFC 8707).
#[derive(Clone)]
pub struct TokenValidator {
    store: Arc<dyn AuthStore>,
    public_url: Option<String>,
    resource_path: String,
}

impl TokenValidator {
    pub fn new(
        store: Arc<dyn AuthStore>,
        public_url: Option<String>,
        resource_path: impl Into<String>,
    ) -> Self {
        Self {
            store,
            public_url,
            resource_path: resource_path.into(),
        }
    }

    /// Returns the caller, or `None` when the token is absent, unknown,
    /// expired or issued for a different audience.
    pub async fn validate(&self, headers: &HeaderMap) -> Option<AuthContext> {
        let token = bearer_token(headers)?;
        let record = self
            .store
            .find_access_token(&hash_token(token))
            .await
            .map_err(|e| tracing::error!("token lookup failed: {e}"))
            .ok()??;
        let audience = format!(
            "{}{}",
            base_url(self.public_url.as_deref(), headers),
            self.resource_path
        );
        if record.expires_at <= now() || !ct_eq(&record.audience, &audience) {
            return None;
        }
        Some(AuthContext {
            user_id: record.user_id,
            client_id: record.client_id,
            scopes: record.scopes,
            audience: record.audience,
        })
    }

    /// The 401 response mandated by the MCP authorization spec.
    pub fn unauthorized(&self, headers: &HeaderMap) -> Response {
        let base = base_url(self.public_url.as_deref(), headers);
        let mut challenge =
            format!("Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource\"");
        if bearer_token(headers).is_some() {
            challenge.push_str(", error=\"invalid_token\"");
        }
        let mut response = (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
        if let Ok(value) = HeaderValue::from_str(&challenge) {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
        response
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

/// Axum middleware. Apply with
/// `axum::middleware::from_fn_with_state(validator, require_bearer)`.
/// On success an [`AuthContext`] is available as a request extension.
pub async fn require_bearer(
    State(validator): State<TokenValidator>,
    mut request: Request,
    next: Next,
) -> Response {
    match validator.validate(request.headers()).await {
        Some(context) => {
            request.extensions_mut().insert(context);
            next.run(request).await
        }
        None => validator.unauthorized(request.headers()),
    }
}
