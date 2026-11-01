//! The account and security page: password, TOTP, passkeys, sessions and
//! connected OAuth clients.

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Form, Json, Router,
};
use folio_auth::{
    crypto::{hash_password, hash_token},
    mfa,
    model::SOURCE_LOCAL,
    passkey::{Passkey, RegisterPublicKeyCredential},
    server::now,
    PasskeyRecord, User,
};
use serde::Deserialize;
use serde_json::json;

use super::{
    auth::{set_cookie, with_cookies, AuthState},
    mfa::{credential_id_string, parse_user_uuid, recovery_hash, verify_second_factor},
    view::{Nav, Sidebar},
};
use crate::{error::ApiResult, state::AppState};

const MIN_PASSWORD_LEN: usize = 12;
const MAX_PASSWORD_LEN: usize = 1024;
const MAX_PASSKEYS: usize = 20;
const MAX_NAME_LEN: usize = 64;

pub fn router(auth: AuthState) -> Router<AppState> {
    Router::new()
        .route("/settings/security", get(page))
        .route("/settings/security/password", post(change_password))
        .route("/settings/security/totp/start", post(totp_start))
        .route("/settings/security/totp/confirm", post(totp_confirm))
        .route("/settings/security/totp/cancel", post(totp_cancel))
        .route("/settings/security/totp/disable", post(totp_disable))
        .route("/settings/security/totp/recovery", post(totp_recovery))
        .route(
            "/settings/security/passkeys/register/start",
            post(passkey_register_start),
        )
        .route(
            "/settings/security/passkeys/register/finish",
            post(passkey_register_finish),
        )
        .route(
            "/settings/security/passkeys/{id}/rename",
            post(passkey_rename),
        )
        .route(
            "/settings/security/passkeys/{id}/delete",
            post(passkey_delete),
        )
        .route(
            "/settings/security/sessions/revoke-all",
            post(sessions_revoke_all),
        )
        .route(
            "/settings/security/clients/{id}/revoke",
            post(client_revoke),
        )
        .layer(Extension(auth))
}

struct Notice {
    text: &'static str,
    danger: bool,
}

struct PasskeyView {
    id: String,
    name: String,
    created: String,
    last_used: String,
}

struct SessionView {
    current: bool,
    agent: String,
    created: String,
    expires: String,
}

struct ClientView {
    id: String,
    name: String,
    expires: String,
}

#[derive(Template)]
#[template(path = "settings_security.html")]
struct SecurityTemplate {
    sidebar: Sidebar,
    title: String,
    csrf: String,
    username: String,
    local: bool,
    notice: Option<Notice>,
    recovery_codes: Vec<String>,
    totp_enabled: bool,
    totp_pending: bool,
    qr_svg: String,
    secret_key: String,
    recovery_left: i64,
    passkeys_available: bool,
    passkeys: Vec<PasskeyView>,
    sessions: Vec<SessionView>,
    clients: Vec<ClientView>,
}

fn notice(key: &str) -> Option<Notice> {
    let (text, danger) = match key {
        "password" => ("Password changed. Other devices were signed out.", false),
        "totp-on" => ("Two-factor sign-in is on.", false),
        "totp-off" => ("Two-factor sign-in is off.", false),
        "passkey-renamed" => ("Passkey renamed.", false),
        "passkey-deleted" => ("Passkey deleted.", false),
        "client-revoked" => ("The app was disconnected.", false),
        "bad-password" => ("The current password is not right.", true),
        "password-mismatch" => ("The two new passwords differ.", true),
        "password-length" => ("Use a password of 12 to 1024 characters.", true),
        "bad-code" => ("That code is not right.", true),
        "bad-factor" => ("The password or the code is not right.", true),
        "locked" => ("Too many wrong attempts. Wait a while and try again.", true),
        "totp-exists" => ("Two-factor sign-in is already on.", true),
        "bad-name" => ("Give the passkey a name of up to 64 characters.", true),
        _ => return None,
    };
    Some(Notice { text, danger })
}

fn at(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn agent_label(agent: Option<&str>) -> String {
    let agent = agent.unwrap_or("");
    let os = ["iPhone", "iPad", "Android", "Mac OS X", "Windows", "Linux"]
        .into_iter()
        .find(|os| agent.contains(os));
    let browser = ["Edg/", "Firefox/", "Chrome/", "Safari/"]
        .into_iter()
        .zip(["Edge", "Firefox", "Chrome", "Safari"])
        .find(|(m, _)| agent.contains(m))
        .map(|(_, name)| name);
    match (browser, os) {
        (Some(b), Some(o)) => format!("{b} on {o}"),
        (Some(b), None) => b.to_string(),
        (None, Some(o)) => o.to_string(),
        (None, None) => "Unknown device".to_string(),
    }
}

/// The session id, user and the CSRF token of the caller.
struct Caller {
    session_id: String,
    user: User,
    csrf: String,
}

async fn caller(auth: &AuthState, headers: &HeaderMap) -> Result<Caller, Response> {
    match auth.session(headers).await {
        Some((session_id, user)) => Ok(Caller {
            csrf: auth.csrf_session_token(&session_id),
            session_id,
            user,
        }),
        None => Err(Redirect::to("/login?next=%2Fsettings%2Fsecurity").into_response()),
    }
}

/// A caller who may change local credentials, with the form's CSRF token verified.
async fn checked(auth: &AuthState, headers: &HeaderMap, csrf: &str) -> Result<Caller, Response> {
    let caller = caller(auth, headers).await?;
    if !auth.csrf_session_ok(headers, csrf).await || caller.user.auth_source != SOURCE_LOCAL {
        return Err(StatusCode::FORBIDDEN.into_response());
    }
    Ok(caller)
}

fn back(key: &str) -> Response {
    Redirect::to(&format!("/settings/security?n={key}")).into_response()
}

#[derive(Deserialize)]
struct PageQuery {
    n: Option<String>,
}

async fn render_page(
    state: &AppState,
    auth: &AuthState,
    caller: &Caller,
    notice_key: Option<&str>,
    recovery_codes: Vec<String>,
) -> ApiResult<Response> {
    let user = &caller.user;
    let local = user.auth_source == SOURCE_LOCAL;
    let totp = auth.store.get_totp(&user.id).await.ok().flatten();
    let (mut qr_svg, mut secret_key) = (String::new(), String::new());
    let totp_pending = totp.as_ref().is_some_and(|t| !t.confirmed);
    if let Some(record) = totp.as_ref().filter(|t| !t.confirmed) {
        if let Some(seed) = auth.secret_box.open(&record.secret_enc, &user.id) {
            if let Ok(t) = mfa::totp(&seed, &user.username) {
                qr_svg = mfa::otpauth_uri(&t)
                    .and_then(|uri| mfa::qr_svg(&uri))
                    .unwrap_or_default();
                secret_key = mfa::base32_secret(&seed);
            }
        }
    }
    let passkeys = auth
        .store
        .list_passkeys(&user.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|p| PasskeyView {
            last_used: p.last_used_at.map_or_else(
                || "Not used yet".to_string(),
                |t| format!("Last used {}", at(t)),
            ),
            created: at(p.created_at),
            id: p.id,
            name: p.name,
        })
        .collect();
    let current = hash_token(&caller.session_id);
    let sessions = auth
        .store
        .list_sessions(&user.id, now())
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|s| SessionView {
            current: s.id_hash == current,
            agent: agent_label(s.user_agent.as_deref()),
            created: if s.created_at > 0 {
                at(s.created_at)
            } else {
                "earlier".into()
            },
            expires: at(s.expires_at),
        })
        .collect();
    let clients = auth
        .store
        .list_connected_clients(&user.id, now())
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| ClientView {
            id: c.client_id,
            name: c.client_name,
            expires: at(c.expires_at),
        })
        .collect();
    let template = SecurityTemplate {
        sidebar: Sidebar::load(state, Nav::Settings).await?,
        title: "Account and security - Folio".into(),
        csrf: caller.csrf.clone(),
        username: user.username.clone(),
        local,
        notice: notice_key.and_then(notice),
        recovery_codes,
        totp_enabled: totp.as_ref().is_some_and(|t| t.confirmed),
        totp_pending,
        qr_svg,
        secret_key,
        recovery_left: auth.store.count_recovery_codes(&user.id).await.unwrap_or(0),
        passkeys_available: auth.passkeys_enabled(),
        passkeys,
        sessions,
        clients,
    };
    let body = template.render().map_err(|e| {
        tracing::error!("security page failed to render: {e}");
        crate::error::ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not render the page",
        )
    })?;
    let mut response = Html(body).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().expect("static"));
    Ok(response)
}

async fn page(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response {
    let caller = match caller(&auth, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    render_page(&state, &auth, &caller, q.n.as_deref(), Vec::new())
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

/// Verifies the account password, counting a miss against the login limiter.
async fn password_ok(auth: &AuthState, user: &User, password: &str) -> Result<bool, &'static str> {
    let key = user.username.to_lowercase();
    if auth.login_by_user.check(&key).is_err() {
        return Err("locked");
    }
    if password.chars().count() > MAX_PASSWORD_LEN {
        return Ok(false);
    }
    let (password, hash) = (password.to_string(), user.password_hash.clone());
    let ok = auth
        .hash_work(move || folio_auth::crypto::verify_password(&password, &hash))
        .await
        .unwrap_or(false);
    if ok {
        auth.login_by_user.record_success(&key);
    } else {
        auth.login_by_user.record_failure(&key);
    }
    Ok(ok)
}

#[derive(Deserialize)]
struct PasswordForm {
    csrf: String,
    current_password: String,
    new_password: String,
    confirm_password: String,
}

async fn change_password(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Form(form): Form<PasswordForm>,
) -> Response {
    let caller = match checked(&auth, &headers, &form.csrf).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match password_ok(&auth, &caller.user, &form.current_password).await {
        Err(key) => return back(key),
        Ok(false) => return back("bad-password"),
        Ok(true) => {}
    }
    if form.new_password != form.confirm_password {
        return back("password-mismatch");
    }
    let len = form.new_password.chars().count();
    if !(MIN_PASSWORD_LEN..=MAX_PASSWORD_LEN).contains(&len) {
        return back("password-length");
    }
    let new = form.new_password.clone();
    let Some(Ok(hash)) = auth.hash_work(move || hash_password(&new)).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let keep = hash_token(&caller.session_id);
    if auth
        .store
        .set_password_hash(&caller.user.id, &hash)
        .await
        .is_err()
        || auth
            .store
            .delete_user_sessions(&caller.user.id, Some(&keep))
            .await
            .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    back("password")
}

#[derive(Deserialize)]
struct CsrfOnly {
    csrf: String,
}

async fn totp_start(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Form(form): Form<CsrfOnly>,
) -> Response {
    let caller = match checked(&auth, &headers, &form.csrf).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let sealed = auth
        .secret_box
        .seal(&mfa::new_totp_secret(), &caller.user.id);
    match auth
        .store
        .save_pending_totp(&caller.user.id, &sealed, now())
        .await
    {
        Ok(true) => Redirect::to("/settings/security").into_response(),
        Ok(false) => back("totp-exists"),
        Err(e) => {
            tracing::error!("saving the TOTP seed failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
struct CodeForm {
    csrf: String,
    code: String,
}

async fn totp_confirm(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Form(form): Form<CodeForm>,
) -> Response {
    let caller = match checked(&auth, &headers, &form.csrf).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let user = &caller.user;
    if auth.code_by_user.check(&user.id).is_err() {
        return back("locked");
    }
    let record = auth.store.get_totp(&user.id).await.ok().flatten();
    let Some(record) = record.filter(|r| !r.confirmed) else {
        return Redirect::to("/settings/security").into_response();
    };
    let step = auth
        .secret_box
        .open(&record.secret_enc, &user.id)
        .and_then(|seed| mfa::totp(&seed, &user.username).ok())
        .and_then(|t| mfa::check_code(&t, &form.code, now().max(0) as u64, record.last_step));
    let Some(step) = step else {
        auth.code_by_user.record_failure(&user.id);
        return back("bad-code");
    };
    auth.code_by_user.record_success(&user.id);
    let codes = mfa::generate_recovery_codes();
    let hashes = hashes_of(&auth, &user.id, &codes);
    match auth.store.confirm_totp(&user.id, step, &hashes).await {
        Ok(true) => render_page(&state, &auth, &caller, Some("totp-on"), codes)
            .await
            .unwrap_or_else(IntoResponse::into_response),
        Ok(false) => Redirect::to("/settings/security").into_response(),
        Err(e) => {
            tracing::error!("confirming TOTP failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn hashes_of(auth: &AuthState, user_id: &str, codes: &[String]) -> Vec<String> {
    codes
        .iter()
        .map(|c| recovery_hash(auth, user_id, &mfa::normalize_recovery_code(c)))
        .collect()
}

async fn totp_cancel(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Form(form): Form<CsrfOnly>,
) -> Response {
    let caller = match checked(&auth, &headers, &form.csrf).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let pending = auth
        .store
        .get_totp(&caller.user.id)
        .await
        .ok()
        .flatten()
        .is_some_and(|t| !t.confirmed);
    if pending {
        let _ = auth.store.delete_totp(&caller.user.id).await;
    }
    Redirect::to("/settings/security").into_response()
}

#[derive(Deserialize)]
struct PasswordCodeForm {
    csrf: String,
    password: String,
    code: String,
}

/// Password plus a current code, for the actions that weaken or reset the
/// second factor. Both are checked against the lockout counters.
async fn reauthenticate(
    auth: &AuthState,
    headers: &HeaderMap,
    form: &PasswordCodeForm,
) -> Result<Caller, Response> {
    let caller = checked(auth, headers, &form.csrf).await?;
    if auth.code_by_user.check(&caller.user.id).is_err() {
        return Err(back("locked"));
    }
    match password_ok(auth, &caller.user, &form.password).await {
        Err(key) => return Err(back(key)),
        Ok(false) => return Err(back("bad-factor")),
        Ok(true) => {}
    }
    if !verify_second_factor(auth, &caller.user, &form.code).await {
        auth.code_by_user.record_failure(&caller.user.id);
        return Err(back("bad-factor"));
    }
    auth.code_by_user.record_success(&caller.user.id);
    Ok(caller)
}

async fn totp_disable(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Form(form): Form<PasswordCodeForm>,
) -> Response {
    let caller = match reauthenticate(&auth, &headers, &form).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match auth.store.delete_totp(&caller.user.id).await {
        Ok(()) => back("totp-off"),
        Err(e) => {
            tracing::error!("disabling TOTP failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn totp_recovery(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Form(form): Form<PasswordCodeForm>,
) -> Response {
    let caller = match reauthenticate(&auth, &headers, &form).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let codes = mfa::generate_recovery_codes();
    let hashes = hashes_of(&auth, &caller.user.id, &codes);
    if let Err(e) = auth
        .store
        .replace_recovery_codes(&caller.user.id, &hashes)
        .await
    {
        tracing::error!("replacing recovery codes failed: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    render_page(&state, &auth, &caller, None, codes)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

// ---- passkeys ---------------------------------------------------------------

fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// Checks the session, the `X-CSRF` header and that the account is local.
async fn checked_json(auth: &AuthState, headers: &HeaderMap) -> Result<Caller, Response> {
    let submitted = headers
        .get("x-csrf")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    checked(auth, headers, submitted).await
}

async fn passkey_register_start(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
) -> Response {
    let caller = match checked_json(&auth, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let (Some(webauthn), Some(uuid)) = (auth.webauthn.clone(), parse_user_uuid(&caller.user))
    else {
        return json_error(StatusCode::NOT_FOUND, "Passkeys are not available.");
    };
    let existing = auth
        .store
        .list_passkeys(&caller.user.id)
        .await
        .unwrap_or_default();
    if existing.len() >= MAX_PASSKEYS {
        return json_error(StatusCode::CONFLICT, "You have reached the passkey limit.");
    }
    let exclude: Vec<_> = existing
        .iter()
        .filter_map(|r| serde_json::from_str::<Passkey>(&r.data).ok())
        .map(|p| p.cred_id().clone())
        .collect();
    let display = caller
        .user
        .display_name
        .clone()
        .unwrap_or_else(|| caller.user.username.clone());
    match webauthn.start_passkey_registration(
        uuid,
        &caller.user.username,
        &display,
        (!exclude.is_empty()).then_some(exclude),
    ) {
        Ok((challenge, state)) => {
            if !auth.passkey_regs.insert(caller.user.id.clone(), state) {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            Json(challenge).into_response()
        }
        Err(e) => {
            tracing::error!("starting passkey registration failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
struct RegisterFinish {
    name: String,
    credential: RegisterPublicKeyCredential,
}

async fn passkey_register_finish(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Json(body): Json<RegisterFinish>,
) -> Response {
    let caller = match checked_json(&auth, &headers).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let Some(webauthn) = auth.webauthn.clone() else {
        return json_error(StatusCode::NOT_FOUND, "Passkeys are not available.");
    };
    let Some(state) = auth.passkey_regs.take(&caller.user.id) else {
        return json_error(StatusCode::BAD_REQUEST, "The request expired. Try again.");
    };
    let passkey = match webauthn.finish_passkey_registration(&body.credential, &state) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("passkey registration was refused: {e}");
            return json_error(StatusCode::BAD_REQUEST, "That passkey was not accepted.");
        }
    };
    let name = body.name.trim();
    let name = if name.is_empty() { "Passkey" } else { name };
    let Ok(data) = serde_json::to_string(&passkey) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let record = PasskeyRecord {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: caller.user.id.clone(),
        name: name.chars().take(MAX_NAME_LEN).collect(),
        credential_id: credential_id_string(passkey.cred_id().as_ref()),
        data,
        created_at: now(),
        last_used_at: None,
    };
    match auth.store.insert_passkey(record).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(folio_auth::AuthError::Conflict) => {
            json_error(StatusCode::CONFLICT, "That passkey is already registered.")
        }
        Err(e) => {
            tracing::error!("storing the passkey failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
struct RenameForm {
    csrf: String,
    name: String,
}

async fn passkey_rename(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<RenameForm>,
) -> Response {
    let caller = match checked(&auth, &headers, &form.csrf).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let name = form.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LEN {
        return back("bad-name");
    }
    match auth.store.rename_passkey(&caller.user.id, &id, name).await {
        Ok(true) => back("passkey-renamed"),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn passkey_delete(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<CsrfOnly>,
) -> Response {
    let caller = match checked(&auth, &headers, &form.csrf).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    match auth.store.delete_passkey(&caller.user.id, &id).await {
        Ok(true) => back("passkey-deleted"),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

// ---- sessions and clients ---------------------------------------------------

async fn sessions_revoke_all(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Form(form): Form<CsrfOnly>,
) -> Response {
    let Ok(caller) = caller(&auth, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    if !auth.csrf_session_ok(&headers, &form.csrf).await {
        return StatusCode::FORBIDDEN.into_response();
    }
    if auth
        .store
        .delete_user_sessions(&caller.user.id, None)
        .await
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let clear = set_cookie(auth.session_cookie_name(), "", 0, auth.secure());
    with_cookies(Redirect::to("/login").into_response(), [clear])
}

async fn client_revoke(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Path(client_id): Path<String>,
    Form(form): Form<CsrfOnly>,
) -> Response {
    let Ok(caller) = caller(&auth, &headers).await else {
        return Redirect::to("/login").into_response();
    };
    if !auth.csrf_session_ok(&headers, &form.csrf).await {
        return StatusCode::FORBIDDEN.into_response();
    }
    match auth
        .store
        .revoke_client_tokens(&caller.user.id, &client_id)
        .await
    {
        Ok(()) => back("client-revoked"),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
