//! Sign in through an OpenID Connect provider: start and callback.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, Query, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use folio_auth::{
    crypto::{ct_eq, hash_token, random_token},
    external::{sync_external_user, ExternalError},
    oidc::OidcError,
    redirect::safe_next,
    server::now,
    OidcLogin,
};
use serde::Deserialize;

use super::auth::{
    cookie, create_session, error_page_with, is_secure, set_cookie, with_cookies, AuthState,
    NextQuery, SESSION_COOKIE, SESSION_TTL_SECS,
};

const LOGIN_TTL_SECS: i64 = 600;
const START_WINDOW: Duration = Duration::from_secs(60);
const STARTS_PER_WINDOW: u32 = 30;
const MAX_TRACKED_CLIENTS: usize = 10_000;

/// The `__Host-` prefix pins the cookie to this exact host and to https; it is
/// only valid together with `Secure`.
fn browser_cookie_name(secure: bool) -> &'static str {
    if secure {
        "__Host-folio_oidc"
    } else {
        "folio_oidc"
    }
}

/// Caps how often one client address can start a login, since every start
/// stores a row.
#[derive(Default)]
pub(crate) struct StartLimiter(Mutex<HashMap<IpAddr, (Instant, u32)>>);

impl StartLimiter {
    fn allow(&self, ip: IpAddr) -> bool {
        let mut seen = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if seen.len() >= MAX_TRACKED_CLIENTS {
            seen.retain(|_, (since, _)| now.duration_since(*since) < START_WINDOW);
        }
        let entry = seen.entry(ip).or_insert((now, 0));
        if now.duration_since(entry.0) >= START_WINDOW {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= STARTS_PER_WINDOW
    }
}

fn redirect_uri(auth: &AuthState, headers: &HeaderMap) -> String {
    format!("{}/auth/oidc/callback", auth.base(headers))
}

pub(crate) async fn start(State(auth): State<AuthState>, request: Request) -> Response {
    let Some(provider) = auth.oidc.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::from([0, 0, 0, 0]), |c| c.0.ip());
    if !auth.oidc_starts.allow(peer) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let headers = request.headers();
    let q = Query::<NextQuery>::try_from_uri(request.uri())
        .map(|q| q.0)
        .unwrap_or(NextQuery { next: None });
    let secure = is_secure(&auth.base(headers));
    let request = match provider.authorize(&redirect_uri(&auth, headers)).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("starting the OIDC login failed: {e}");
            return error_page_with(
                "Single sign-on is unavailable right now.",
                StatusCode::BAD_GATEWAY,
            );
        }
    };
    let browser = random_token();
    let stored = auth
        .store
        .insert_oidc_login(OidcLogin {
            state_hash: hash_token(&request.state),
            browser_hash: hash_token(&browser),
            nonce: request.nonce,
            pkce_verifier: request.pkce_verifier,
            next: safe_next(q.next.as_deref()),
            expires_at: now() + LOGIN_TTL_SECS,
        })
        .await;
    if let Err(e) = stored {
        tracing::error!("storing the OIDC login failed: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let binding = set_cookie(
        browser_cookie_name(secure),
        &browser,
        LOGIN_TTL_SECS,
        secure,
    );
    with_cookies(
        Redirect::to(request.url.as_str()).into_response(),
        [binding],
    )
}

#[derive(Deserialize)]
pub(crate) struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

pub(crate) async fn callback(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let Some(provider) = auth.oidc.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let secure = is_secure(&auth.base(&headers));
    let clear = set_cookie(browser_cookie_name(secure), "", 0, secure);
    let fail = |message: &str, status: StatusCode| {
        with_cookies(error_page_with(message, status), [clear.clone()])
    };

    let Some(state) = q.state.filter(|s| !s.is_empty()) else {
        return fail("The sign-in request is not valid.", StatusCode::BAD_REQUEST);
    };
    let login = match auth.store.take_oidc_login(&hash_token(&state)).await {
        Ok(Some(login)) => login,
        Ok(None) => {
            return fail(
                "The sign-in request is unknown or has expired. Start again.",
                StatusCode::BAD_REQUEST,
            )
        }
        Err(e) => {
            tracing::error!("reading the OIDC login failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let same_browser = cookie(&headers, browser_cookie_name(secure))
        .is_some_and(|c| ct_eq(&hash_token(&c), &login.browser_hash));
    if login.expires_at <= now() || !same_browser {
        return fail(
            "The sign-in request is unknown or has expired. Start again.",
            StatusCode::BAD_REQUEST,
        );
    }
    if let Some(error) = q.error {
        let shown: String = error.chars().take(64).collect();
        tracing::warn!("the identity provider refused the login: {shown:?}");
        return fail(
            "The identity provider refused the sign-in.",
            StatusCode::FORBIDDEN,
        );
    }
    let Some(code) = q.code.filter(|c| !c.is_empty()) else {
        return fail("The sign-in request is not valid.", StatusCode::BAD_REQUEST);
    };

    let result = match provider
        .complete(
            &redirect_uri(&auth, &headers),
            &code,
            &login.nonce,
            &login.pkce_verifier,
        )
        .await
    {
        Ok(result) => result,
        Err(OidcError::Rejected(reason)) => {
            tracing::warn!("OIDC login rejected: {reason}");
            return fail("The sign-in could not be verified.", StatusCode::FORBIDDEN);
        }
        Err(OidcError::Provider(reason)) => {
            tracing::error!("OIDC login failed: {reason}");
            return fail(
                "Single sign-on is unavailable right now.",
                StatusCode::BAD_GATEWAY,
            );
        }
    };

    let identity = result.identity;
    let known = match auth
        .store
        .find_user_by_external(&identity.source, &identity.external_id)
        .await
    {
        Ok(known) => known,
        Err(e) => {
            tracing::error!("OIDC user lookup failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if known.is_none() && !provider.auto_create_users() {
        return fail(
            "New accounts are not created through single sign-on here.",
            StatusCode::FORBIDDEN,
        );
    }
    // Without trustworthy groups, keep what is stored instead of demoting.
    let synced = match known {
        Some(existing) if !result.groups_known => Ok(existing),
        _ => sync_external_user(&*auth.store, identity, provider.admin_group()).await,
    };
    let user = match synced {
        Ok(user) => user,
        Err(ExternalError::UsernameTaken | ExternalError::Invalid) => {
            return fail(
                "This account can't sign in through single sign-on.",
                StatusCode::FORBIDDEN,
            );
        }
        Err(ExternalError::Store(e)) => {
            tracing::error!("OIDC user sync failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    // A login replaces the session it was started with.
    if let Some(old) = cookie(&headers, SESSION_COOKIE) {
        let _ = auth.store.delete_session(&hash_token(&old)).await;
    }
    let Some(session) = create_session(&auth, &user).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if let Err(e) = auth
        .store
        .set_session_id_token(&hash_token(&session), &result.id_token)
        .await
    {
        tracing::warn!("storing the ID token failed: {e}");
    }
    let session = set_cookie(SESSION_COOKIE, &session, SESSION_TTL_SECS, secure);
    with_cookies(
        Redirect::to(&safe_next(Some(&login.next))).into_response(),
        [clear, session],
    )
}
