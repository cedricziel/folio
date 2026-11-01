//! The second step of a login: authenticator code, recovery code or passkey.

use askama::Template;
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Form, Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use folio_auth::{
    crypto::{hash_token, random_token},
    mfa,
    model::SOURCE_LOCAL,
    passkey::{Passkey, PublicKeyCredential, Uuid},
    redirect::safe_next,
    server::now,
    User,
};
use serde::Deserialize;
use serde_json::json;

use super::auth::{
    cookie, render, set_cookie, start_session, wait_text, with_cookies, AuthState, ClientIp,
    MfaLogin, PasskeyAuth,
};

/// Wrong codes allowed on one pending login before the password is asked again.
const ATTEMPTS_PER_LOGIN: u32 = 5;
const CEREMONY_COOKIE_TTL: i64 = 300;

fn mfa_cookie_name(secure: bool) -> &'static str {
    if secure {
        "__Host-folio_mfa"
    } else {
        "folio_mfa"
    }
}

fn passkey_cookie_name(secure: bool) -> &'static str {
    if secure {
        "__Host-folio_passkey"
    } else {
        "folio_passkey"
    }
}

#[derive(Template)]
#[template(path = "auth/login_code.html")]
struct CodeTemplate {
    csrf: String,
    error: Option<String>,
    passkeys: bool,
    next: String,
}

/// Whether the account has a confirmed authenticator-app enrollment.
pub(crate) async fn has_totp(auth: &AuthState, user_id: &str) -> Result<bool, ()> {
    auth.store
        .get_totp(user_id)
        .await
        .map(|t| t.is_some_and(|t| t.confirmed))
        .map_err(|e| tracing::error!("reading the TOTP enrollment failed: {e}"))
}

/// After a correct password: starts the code step when the account has TOTP.
/// `None` means no second step is needed. Failing to read the enrollment
/// refuses the login rather than skipping the second factor.
pub(crate) async fn challenge(auth: &AuthState, user: &User, next: &str) -> Option<Response> {
    if user.auth_source != SOURCE_LOCAL {
        return None;
    }
    match has_totp(auth, &user.id).await {
        Ok(false) => None,
        Err(()) => Some(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Ok(true) => {
            let token = random_token();
            let stored = auth.mfa_logins.insert(
                hash_token(&token),
                MfaLogin {
                    user_id: user.id.clone(),
                    next: next.to_string(),
                    attempts: 0,
                },
            );
            if !stored {
                return Some(StatusCode::SERVICE_UNAVAILABLE.into_response());
            }
            let cookie = set_cookie(
                mfa_cookie_name(auth.secure()),
                &token,
                CEREMONY_COOKIE_TTL,
                auth.secure(),
            );
            Some(with_cookies(
                Redirect::to("/login/code").into_response(),
                [cookie],
            ))
        }
    }
}

/// Checks an authenticator code or a recovery code and consumes it. Used by
/// the login step and by the security page's password-plus-code actions.
pub(crate) async fn verify_second_factor(auth: &AuthState, user: &User, input: &str) -> bool {
    let Ok(Some(record)) = auth.store.get_totp(&user.id).await else {
        return false;
    };
    if !record.confirmed {
        return false;
    }
    let digits: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit()) {
        let Some(seed) = auth.secret_box.open(&record.secret_enc, &user.id) else {
            tracing::error!(
                "the TOTP seed of a user cannot be decrypted; is auth.secret_key unchanged?"
            );
            return false;
        };
        let Ok(totp) = mfa::totp(&seed, &user.username) else {
            return false;
        };
        let Some(step) = mfa::check_code(&totp, &digits, now().max(0) as u64, record.last_step)
        else {
            return false;
        };
        return auth
            .store
            .advance_totp_step(&user.id, step)
            .await
            .unwrap_or(false);
    }
    let normalized = mfa::normalize_recovery_code(input);
    if normalized.len() != 12 {
        return false;
    }
    auth.store
        .use_recovery_code(&user.id, &recovery_hash(auth, &user.id, &normalized))
        .await
        .unwrap_or(false)
}

pub(crate) fn recovery_hash(auth: &AuthState, user_id: &str, normalized: &str) -> String {
    auth.secret_box
        .keyed_hash(&format!("recovery:{user_id}:{normalized}"))
}

fn pending_key(auth: &AuthState, headers: &HeaderMap) -> Option<String> {
    cookie(headers, mfa_cookie_name(auth.secure())).map(|t| hash_token(&t))
}

pub(crate) async fn code_page(State(auth): State<AuthState>, headers: HeaderMap) -> Response {
    let live =
        pending_key(&auth, &headers).and_then(|k| auth.mfa_logins.peek(&k, |p| p.next.clone()));
    let Some(next) = live else {
        return Redirect::to("/login").into_response();
    };
    let (csrf, set) = auth.csrf_pre_token(&headers);
    with_cookies(
        render(
            CodeTemplate {
                csrf,
                error: None,
                passkeys: auth.passkeys_enabled(),
                next,
            },
            StatusCode::OK,
        ),
        set,
    )
}

#[derive(Deserialize)]
pub(crate) struct CodeForm {
    csrf: String,
    code: String,
}

fn too_many(seconds: u64) -> String {
    format!("Too many wrong codes. Try again in {}.", wait_text(seconds))
}

pub(crate) async fn code_submit(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    ClientIp(ip): ClientIp,
    Form(form): Form<CodeForm>,
) -> Response {
    if !auth.csrf_pre_ok(&headers, &form.csrf) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(key) = pending_key(&auth, &headers) else {
        return Redirect::to("/login").into_response();
    };
    let Some((pending, remaining)) = auth.mfa_logins.take_with_remaining(&key) else {
        return Redirect::to("/login").into_response();
    };
    let page = |error: String, status: StatusCode, next: &str| {
        let mut response = render(
            CodeTemplate {
                csrf: form.csrf.clone(),
                error: Some(error),
                passkeys: auth.passkeys_enabled(),
                next: next.to_string(),
            },
            status,
        );
        if status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(30u64));
        }
        response
    };

    let locked = auth
        .code_by_ip
        .check(&ip)
        .err()
        .max(auth.code_by_user.check(&pending.user_id).err());
    if let Some(seconds) = locked {
        let response = page(
            too_many(seconds),
            StatusCode::TOO_MANY_REQUESTS,
            &pending.next,
        );
        auth.mfa_logins.restore(key, pending, remaining);
        return response;
    }

    let user = auth
        .store
        .find_user_by_id(&pending.user_id)
        .await
        .ok()
        .flatten();
    let ok = match &user {
        Some(user) => verify_second_factor(&auth, user, &form.code).await,
        None => false,
    };
    let Some(user) = user.filter(|_| ok) else {
        auth.code_by_ip.record_failure(&ip);
        auth.code_by_user.record_failure(&pending.user_id);
        let attempts = pending.attempts + 1;
        if attempts >= ATTEMPTS_PER_LOGIN {
            let clear = set_cookie(mfa_cookie_name(auth.secure()), "", 0, auth.secure());
            return with_cookies(
                super::auth::error_page_with(
                    "Too many wrong codes. Sign in again.",
                    StatusCode::TOO_MANY_REQUESTS,
                ),
                [clear],
            );
        }
        let next = pending.next.clone();
        auth.mfa_logins.restore(
            key,
            MfaLogin {
                attempts,
                ..pending
            },
            remaining,
        );
        return page(
            "That code is not right. Check the app and try again.".into(),
            StatusCode::UNAUTHORIZED,
            &next,
        );
    };
    auth.code_by_user.record_success(&pending.user_id);
    finish_login(&auth, &headers, &user, &pending.next, false).await
}

/// Ends the pending state, rotates the session id and signs the user in.
async fn finish_login(
    auth: &AuthState,
    headers: &HeaderMap,
    user: &User,
    next: &str,
    json: bool,
) -> Response {
    let secure = auth.secure();
    if let Some(id) = cookie(headers, auth.session_cookie_name()) {
        let _ = auth.store.delete_session(&hash_token(&id)).await;
    }
    let Some(session) = start_session(auth, user).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let cookies = [
        session,
        set_cookie(mfa_cookie_name(secure), "", 0, secure),
        set_cookie(passkey_cookie_name(secure), "", 0, secure),
    ];
    let body = if json {
        Json(json!({ "redirect": next })).into_response()
    } else {
        Redirect::to(next).into_response()
    };
    with_cookies(body, cookies)
}

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

pub(crate) async fn passkey_start(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    ClientIp(ip): ClientIp,
) -> Response {
    let Some(webauthn) = auth.webauthn.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !auth.local_login_allowed() || !auth.is_same_origin(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Err(seconds) = auth.passkey_starts.check(&ip) {
        let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        return response;
    }
    let mfa_user =
        pending_key(&auth, &headers).and_then(|k| auth.mfa_logins.peek(&k, |p| p.user_id.clone()));
    let Ok((challenge, state)) = webauthn.start_discoverable_authentication() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let token = random_token();
    if !auth
        .passkey_auths
        .insert(hash_token(&token), PasskeyAuth { state, mfa_user })
    {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let cookie = set_cookie(
        passkey_cookie_name(auth.secure()),
        &token,
        CEREMONY_COOKIE_TTL,
        auth.secure(),
    );
    with_cookies(Json(challenge).into_response(), [cookie])
}

#[derive(Deserialize)]
pub(crate) struct PasskeyFinish {
    credential: PublicKeyCredential,
    next: Option<String>,
}

pub(crate) async fn passkey_finish(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    ClientIp(ip): ClientIp,
    Json(body): Json<PasskeyFinish>,
) -> Response {
    let Some(webauthn) = auth.webauthn.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !auth.local_login_allowed() || !auth.is_same_origin(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Err(seconds) = auth.code_by_ip.check(&ip) {
        let mut response = json_error(
            StatusCode::TOO_MANY_REQUESTS,
            &format!(
                "Too many failed attempts. Try again in {}.",
                wait_text(seconds)
            ),
        );
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        return response;
    }
    let state = cookie(&headers, passkey_cookie_name(auth.secure()))
        .and_then(|t| auth.passkey_auths.take(&hash_token(&t)));
    let Some(state) = state else {
        return json_error(StatusCode::BAD_REQUEST, "The sign-in expired. Start again.");
    };

    let fail = || {
        auth.code_by_ip.record_failure(&ip);
        json_error(StatusCode::UNAUTHORIZED, "That passkey was not accepted.")
    };
    let Ok((user_uuid, credential_id)) =
        webauthn.identify_discoverable_authentication(&body.credential)
    else {
        return fail();
    };
    let credential_id = credential_id.to_vec();
    let user = auth
        .store
        .find_user_by_id(&user_uuid.to_string())
        .await
        .ok()
        .flatten()
        .filter(|u| u.auth_source == SOURCE_LOCAL)
        .filter(|u| state.mfa_user.as_ref().is_none_or(|m| *m == u.id));
    let Some(user) = user else {
        return fail();
    };
    let records = auth.store.list_passkeys(&user.id).await.unwrap_or_default();
    let mut passkeys: Vec<(String, Passkey)> = records
        .into_iter()
        .filter_map(|r| Some((r.id, serde_json::from_str(&r.data).ok()?)))
        .collect();
    let Some((id, passkey)) = passkeys
        .iter_mut()
        .find(|(_, p)| p.cred_id().as_slice() == credential_id.as_slice())
    else {
        return fail();
    };
    let Ok(result) =
        webauthn.finish_discoverable_authentication_v2(&body.credential, state.state, &*passkey)
    else {
        return fail();
    };
    passkey.update_credential(&result);
    if let Ok(data) = serde_json::to_string(passkey) {
        let _ = auth.store.touch_passkey(id, &data, now()).await;
    }
    auth.code_by_ip.record_success(&ip);

    let next = pending_key(&auth, &headers)
        .and_then(|k| auth.mfa_logins.take(&k))
        .map(|p| p.next)
        .unwrap_or_else(|| safe_next(body.next.as_deref()));
    finish_login(&auth, &headers, &user, &next, true).await
}

/// Encodes a credential id the way it is stored.
pub(crate) fn credential_id_string(id: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(id)
}

pub(crate) fn parse_user_uuid(user: &User) -> Option<Uuid> {
    Uuid::parse_str(&user.id).ok()
}
