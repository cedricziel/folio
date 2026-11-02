//! Trusted-header login for deployments behind an SSO gateway.

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use folio_auth::{
    crypto::hash_token,
    external::{sync_external_user, ExternalError},
    model::SOURCE_FORWARD,
};

use crate::routes::auth::{cookie, create_session, set_cookie, AuthState};

/// Requests that authenticate with OAuth or bearer tokens. They bypass the
/// gateway, so identity headers on them can only be forged.
fn is_machine_request(request: &Request) -> bool {
    let path = request.uri().path();
    let under = |prefix: &str| {
        path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|r| r.starts_with('/'))
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
    let proven = trusted && forward.secret_ok(request.headers());
    if !proven && forward.has_identity(request.headers()) {
        tracing::warn!(
            peer = ?peer,
            trusted_peer = trusted,
            "ignoring forward-auth headers from a peer that is not a verified trusted proxy"
        );
    }
    if !proven || is_machine_request(&request) {
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
        return end_forward_session(&auth, request, next).await;
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
        Some(current) if current.id == user.id => true,
        Some(_) if !accepts_html(request.headers()) => {
            return (
                StatusCode::UNAUTHORIZED,
                "The gateway identity changed. Reload the page to sign in again.",
            )
                .into_response();
        }
        _ => false,
    };
    // Only page loads can store the cookie; other cookieless requests would
    // each leave a session row behind.
    if has_session || !accepts_html(request.headers()) {
        return next.run(request).await;
    }

    let ttl = forward.session_ttl_secs;
    let Some(session_id) = create_session(&auth, &user, ttl).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let name = auth.session_cookie_name();
    set_request_session(request.headers_mut(), name, Some(&session_id));
    let set = set_cookie(name, &session_id, ttl, auth.secure());

    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&set) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

fn accepts_html(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.contains("text/html"))
}

/// Replaces the session cookie in the request's `Cookie` header (or removes
/// it with `None`), keeping other cookies.
fn set_request_session(headers: &mut HeaderMap, name: &str, session_id: Option<&str>) {
    let prefix = format!("{name}=");
    let merged = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .filter(|c| !c.is_empty() && !c.starts_with(&prefix))
        .map(String::from)
        .chain(session_id.map(|id| format!("{prefix}{id}")))
        .collect::<Vec<_>>()
        .join("; ");
    headers.remove(header::COOKIE);
    if let Ok(value) = HeaderValue::from_str(&merged) {
        if !merged.is_empty() {
            headers.insert(header::COOKIE, value);
        }
    }
}

/// A trusted request without identity means the proxy no longer vouches for
/// the browser (signed out upstream): a session that forward auth created is
/// deleted so the cookie cannot outlive the gateway login.
async fn end_forward_session(auth: &AuthState, mut request: Request, next: Next) -> Response {
    let name = auth.session_cookie_name();
    let ended = match (
        cookie(request.headers(), name),
        auth.current_user(request.headers()).await,
    ) {
        (Some(id), Some(user)) if user.auth_source == SOURCE_FORWARD => {
            if let Err(e) = auth.store.delete_session(&hash_token(&id)).await {
                tracing::error!("session delete failed: {e}");
            }
            set_request_session(request.headers_mut(), name, None);
            true
        }
        _ => false,
    };
    let mut response = next.run(request).await;
    if ended {
        if let Ok(value) = HeaderValue::from_str(&set_cookie(name, "", 0, auth.secure())) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}
