//! OpenID Connect login: redirect to the provider and handle its callback.

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use folio_auth::{
    crypto::{ct_eq, hash_token, pkce_challenge, random_token},
    external::{sync_external_user, ExternalError, ExternalIdentity},
    redirect::safe_next,
    server::now,
    OidcFlow,
};
use serde::Deserialize;

use super::auth::{
    create_session, error_page, error_page_with, is_secure, set_cookie, AuthState, SESSION_COOKIE,
    SESSION_TTL_SECS,
};

const STATE_COOKIE: &str = "folio_oidc";
const FLOW_TTL_SECS: i64 = 600;

pub fn router<S>(state: AuthState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/auth/oidc/login", get(start))
        .route("/auth/oidc/callback", get(callback))
        .with_state(state)
}

#[derive(Deserialize)]
struct StartQuery {
    next: Option<String>,
}

async fn start(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Query(q): Query<StartQuery>,
) -> Response {
    let Some(oidc) = auth.oidc.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let base = auth.base(&headers);
    let state = random_token();
    let nonce = random_token();
    let verifier = random_token();
    let url = match oidc
        .authorization_url(
            &format!("{base}/auth/oidc/callback"),
            &state,
            &nonce,
            &pkce_challenge(&verifier),
        )
        .await
    {
        Ok(url) => url,
        Err(e) => {
            tracing::error!("OIDC discovery failed: {e}");
            return error_page_with(
                "The sign-in service can't be reached right now.",
                StatusCode::BAD_GATEWAY,
            );
        }
    };
    let flow = OidcFlow {
        state_hash: hash_token(&state),
        nonce,
        code_verifier: verifier,
        next: safe_next(q.next.as_deref()),
        expires_at: now() + FLOW_TTL_SECS,
    };
    if let Err(e) = auth.store.insert_oidc_flow(flow).await {
        tracing::error!("storing the OIDC flow failed: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    // Binds the flow to this browser: the callback must present the same state.
    let cookie = set_cookie(STATE_COOKIE, &state, FLOW_TTL_SECS, is_secure(&base));
    let mut response = Redirect::to(&url).into_response();
    if let Ok(value) = header::HeaderValue::from_str(&cookie) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn callback(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> Response {
    let Some(oidc) = auth.oidc.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let base = auth.base(&headers);
    let secure = is_secure(&base);
    let clear_state = set_cookie(STATE_COOKIE, "", 0, secure);
    let fail = |message: &str, status: StatusCode| {
        let mut response = error_page_with(message, status);
        if let Ok(v) = header::HeaderValue::from_str(&clear_state) {
            response.headers_mut().append(header::SET_COOKIE, v);
        }
        response
    };

    if let Some(error) = &q.error {
        tracing::warn!("OIDC provider returned an error: {error}");
        return fail(
            "The sign-in was cancelled or refused.",
            StatusCode::BAD_REQUEST,
        );
    }
    let (Some(code), Some(state)) = (q.code.as_deref(), q.state.as_deref()) else {
        return fail(
            "The sign-in response is incomplete.",
            StatusCode::BAD_REQUEST,
        );
    };
    let bound = super::auth::cookie(&headers, STATE_COOKIE).is_some_and(|c| ct_eq(&c, state));
    if !bound {
        return fail(
            "This sign-in didn't start in this browser.",
            StatusCode::BAD_REQUEST,
        );
    }
    let flow = match auth.store.take_oidc_flow(&hash_token(state)).await {
        Ok(Some(flow)) if flow.expires_at > now() => flow,
        Ok(_) => {
            return fail(
                "This sign-in has expired. Start again.",
                StatusCode::BAD_REQUEST,
            )
        }
        Err(e) => {
            tracing::error!("loading the OIDC flow failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let claims = match oidc
        .exchange(
            code,
            &flow.code_verifier,
            &format!("{base}/auth/oidc/callback"),
            &flow.nonce,
        )
        .await
    {
        Ok(claims) => claims,
        Err(e) => {
            tracing::warn!("OIDC login rejected: {e}");
            return fail(
                "The sign-in could not be verified.",
                StatusCode::BAD_REQUEST,
            );
        }
    };

    // (issuer, subject) is the account key.
    let source = format!("oidc:{}", oidc.config.issuer.trim_end_matches('/'));
    if !oidc.config.auto_create_users
        && !matches!(
            auth.store
                .find_user_by_external(&source, &claims.subject)
                .await,
            Ok(Some(_))
        )
    {
        return fail(
            "This account hasn't been set up in Folio yet. Ask the owner to add it.",
            StatusCode::FORBIDDEN,
        );
    }
    let username = claims
        .preferred_username
        .clone()
        .or_else(|| claims.email.clone())
        .unwrap_or_else(|| claims.subject.clone());
    let identity = ExternalIdentity {
        source,
        external_id: claims.subject,
        username,
        email: claims.email,
        display_name: claims.name,
        groups: claims.groups,
    };
    let user = match sync_external_user(&*auth.store, identity, oidc.config.admin_group.as_deref())
        .await
    {
        Ok(user) => user,
        Err(ExternalError::UsernameTaken) => {
            return fail(
                "That username already belongs to another Folio account.",
                StatusCode::FORBIDDEN,
            );
        }
        Err(ExternalError::Invalid) => {
            return fail(
                "The provider sent an unusable username.",
                StatusCode::FORBIDDEN,
            );
        }
        Err(ExternalError::Store(e)) => {
            tracing::error!("OIDC user sync failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Some(session_id) = create_session(&auth, &user).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = Redirect::to(&flow.next).into_response();
    for cookie in [
        set_cookie(SESSION_COOKIE, &session_id, SESSION_TTL_SECS, secure),
        clear_state,
    ] {
        if let Ok(v) = header::HeaderValue::from_str(&cookie) {
            response.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    response
}
