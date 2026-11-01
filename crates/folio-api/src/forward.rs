//! Trusted-header login for deployments behind an SSO gateway.

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use folio_auth::external::{sync_external_user, ExternalError};

use crate::routes::auth::{
    create_session, is_secure, set_cookie, AuthState, SESSION_COOKIE, SESSION_TTL_SECS,
};

/// Requests that authenticate with OAuth or bearer tokens. They bypass the
/// gateway, so identity headers on them can only be forged.
fn is_machine_request(request: &Request) -> bool {
    let path = request.uri().path();
    let under = |prefix: &str| {
        path == prefix || path.strip_prefix(prefix).is_some_and(|r| r.starts_with('/'))
    };
    if matches!(path, "/oauth/token" | "/oauth/register" | "/oauth/revoke")
        || under("/.well-known")
        || under("/mcp")
    {
        return true;
    }
    under("/api")
        && request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.len() > 6 && v[..6].eq_ignore_ascii_case("bearer"))
}

/// Logs in the user named by the forward-auth headers.
///
/// Headers are honoured only when the TCP peer is in `trusted_proxies`. Every
/// configured identity header is removed from the request either way, so later
/// layers never see an unverified value. For a trusted request the user
/// is linked or created, their groups are refreshed, and a normal session
/// cookie is set (and injected into the request), so every later layer and
/// handler sees an ordinary logged-in user. Install it outside
/// [`crate::enforce_auth`].
pub async fn forward_auth(
    State(auth): State<AuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(forward) = auth.forward.clone() else {
        return next.run(request).await;
    };
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    let trusted = peer.is_some_and(|ip| forward.is_trusted(ip));
    if !trusted && forward.has_identity(request.headers()) {
        tracing::warn!(
            peer = ?peer,
            "ignoring forward-auth headers from a peer that is not a trusted proxy"
        );
    }
    if !trusted || is_machine_request(&request) {
        forward.strip(request.headers_mut());
        return next.run(request).await;
    }
    if forward.has_repeated_header(request.headers()) {
        tracing::warn!("rejecting a request with a repeated forward-auth header");
        return (
            StatusCode::BAD_REQUEST,
            "Repeated identity headers are not accepted.",
        )
            .into_response();
    }
    let identity = forward.identity(request.headers());
    forward.strip(request.headers_mut());
    let Some(identity) = identity else {
        return next.run(request).await;
    };

    let user = match sync_external_user(&*auth.store, identity).await {
        Ok(user) => user,
        Err(ExternalError::UsernameTaken | ExternalError::Invalid) => {
            return (
                StatusCode::FORBIDDEN,
                "This account can't sign in through the gateway.",
            )
                .into_response();
        }
        Err(ExternalError::Store(e)) => {
            tracing::error!("forward auth failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let has_session = match auth.current_user(request.headers()).await {
        Some(current) => current.id == user.id,
        None => false,
    };
    if has_session {
        return next.run(request).await;
    }

    let Some(session_id) = create_session(&auth, &user).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let others: Vec<String> = request
        .headers()
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .filter(|c| !c.is_empty() && !c.starts_with(&format!("{SESSION_COOKIE}=")))
        .map(String::from)
        .collect();
    let merged = others
        .into_iter()
        .chain([format!("{SESSION_COOKIE}={session_id}")])
        .collect::<Vec<_>>()
        .join("; ");
    if let Ok(value) = HeaderValue::from_str(&merged) {
        request.headers_mut().insert(header::COOKIE, value);
    }
    let secure = is_secure(&auth.base(request.headers()));
    let set = set_cookie(SESSION_COOKIE, &session_id, SESSION_TTL_SECS, secure);

    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&set) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}
