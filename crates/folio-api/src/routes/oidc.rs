//! Sign in through an OpenID Connect provider: start and callback.

use axum::{
    extract::{Query, State},
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

const BROWSER_COOKIE: &str = "folio_oidc";
const LOGIN_TTL_SECS: i64 = 600;

fn redirect_uri(auth: &AuthState, headers: &HeaderMap) -> String {
    format!("{}/auth/oidc/callback", auth.base(headers))
}

pub(crate) async fn start(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Query(q): Query<NextQuery>,
) -> Response {
    let Some(provider) = auth.oidc.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let secure = is_secure(&auth.base(&headers));
    let request = match provider.authorize(&redirect_uri(&auth, &headers)).await {
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
    let binding = set_cookie(BROWSER_COOKIE, &browser, LOGIN_TTL_SECS, secure);
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
    let clear = set_cookie(BROWSER_COOKIE, "", 0, secure);
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
    let same_browser = cookie(&headers, BROWSER_COOKIE)
        .is_some_and(|c| ct_eq(&hash_token(&c), &login.browser_hash));
    if login.expires_at <= now() || !same_browser {
        return fail(
            "The sign-in request is unknown or has expired. Start again.",
            StatusCode::BAD_REQUEST,
        );
    }
    if let Some(error) = q.error {
        tracing::warn!("the identity provider refused the login: {error}");
        return fail(
            "The identity provider refused the sign-in.",
            StatusCode::FORBIDDEN,
        );
    }
    let Some(code) = q.code.filter(|c| !c.is_empty()) else {
        return fail("The sign-in request is not valid.", StatusCode::BAD_REQUEST);
    };

    let identity = match provider
        .complete(
            &redirect_uri(&auth, &headers),
            &code,
            &login.nonce,
            &login.pkce_verifier,
        )
        .await
    {
        Ok(identity) => identity,
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

    if !provider.auto_create_users() {
        let known = auth
            .store
            .find_user_by_external(&identity.source, &identity.external_id)
            .await;
        if !matches!(known, Ok(Some(_))) {
            return fail(
                "New accounts are not created through single sign-on here.",
                StatusCode::FORBIDDEN,
            );
        }
    }
    let user = match sync_external_user(&*auth.store, identity, provider.admin_group()).await {
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
    let Some(session) = create_session(&auth, &user).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let session = set_cookie(SESSION_COOKIE, &session, SESSION_TTL_SECS, secure);
    with_cookies(
        Redirect::to(&safe_next(Some(&login.next))).into_response(),
        [clear, session],
    )
}
