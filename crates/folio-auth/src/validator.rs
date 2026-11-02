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

/// Origin used when no public URL is configured. Folio refuses to start
/// without one when it is reachable beyond loopback, so this only ever shows
/// in embedded use and tests.
const FALLBACK_BASE: &str = "http://localhost";

/// Public base URL without trailing slash. It comes from configuration only:
/// `Host` and `X-Forwarded-*` are client-controlled and must never decide the
/// issuer, token audiences or cookie flags.
pub fn base_url(configured: Option<&str>) -> String {
    configured
        .filter(|u| !u.is_empty())
        .unwrap_or(FALLBACK_BASE)
        .trim_end_matches('/')
        .to_string()
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
            base_url(self.public_url.as_deref()),
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
        let base = base_url(self.public_url.as_deref());
        let mut challenge = format!(
            "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource{}\"",
            self.resource_path
        );
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
