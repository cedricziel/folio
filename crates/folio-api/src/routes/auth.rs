//! Login, setup and the OAuth 2.1 authorization server endpoints.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{FromRef, FromRequestParts, Query, RawQuery, Request, State},
    http::{header, request::Parts, HeaderMap, HeaderName, HeaderValue, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use url::{form_urlencoded, Url};

use folio_auth::{
    base_url,
    crypto::{ct_eq, hash_password, hash_token, random_token, verify_password},
    model::SOURCE_LOCAL,
    redirect::safe_next,
    server::{
        authorization_server_metadata, now, protected_resource_metadata, AuthorizeFailure,
        AuthorizeParams, OAuthError, OAuthServer, RegistrationRequest, TokenRequest,
        ValidatedAuthorize,
    },
    AuthStore, ForwardAuth, Group, OidcClient, Session, TokenValidator, User,
};

pub(crate) const SESSION_COOKIE: &str = "folio_session";
const CSRF_COOKIE: &str = "folio_csrf";
pub(crate) const SESSION_TTL_SECS: i64 = 14 * 24 * 3600;
const MIN_PASSWORD_LEN: usize = 12;

/// State for the auth routes and the [`RequireUser`] extractor.
#[derive(Clone)]
pub struct AuthState {
    pub store: Arc<dyn AuthStore>,
    pub server: OAuthServer,
    pub public_url: Option<String>,
    /// When false, [`enforce_auth`] lets every request through.
    pub required: bool,
    /// OpenID Connect login through an external provider.
    pub oidc: Option<Arc<OidcClient>>,
    /// Trusted-header login, see [`crate::forward::forward_auth`].
    pub forward: Option<Arc<ForwardAuth>>,
}

impl AuthState {
    pub fn new(store: Arc<dyn AuthStore>, public_url: Option<String>) -> Self {
        Self {
            server: OAuthServer::new(store.clone()),
            store,
            public_url,
            required: true,
            forward: None,
            oidc: None,
        }
    }

    pub fn with_oidc(mut self, oidc: OidcClient) -> Self {
        self.oidc = Some(Arc::new(oidc));
        self
    }

    pub fn with_forward(mut self, forward: ForwardAuth) -> Self {
        self.forward = Some(Arc::new(forward));
        self
    }

    pub fn with_required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Validator for a protected resource path such as `/mcp`.
    /// The user's groups in both namespaces, resolved on demand so callers
    /// (session or token holders alike) always see current memberships.
    pub async fn effective_groups(&self, user_id: &str) -> Vec<Group> {
        self.store.user_groups(user_id).await.unwrap_or_else(|e| {
            tracing::error!("group lookup failed: {e}");
            Vec::new()
        })
    }

    pub fn validator(&self, resource_path: &str) -> TokenValidator {
        TokenValidator::new(self.store.clone(), self.public_url.clone(), resource_path)
    }

    pub(crate) fn base(&self, headers: &HeaderMap) -> String {
        base_url(self.public_url.as_deref(), headers)
    }

    pub(crate) async fn current_user(&self, headers: &HeaderMap) -> Option<User> {
        let id = cookie(headers, SESSION_COOKIE)?;
        let session = self.store.find_session(&hash_token(&id)).await.ok()??;
        if session.expires_at <= now() {
            return None;
        }
        self.store.find_user_by_id(&session.user_id).await.ok()?
    }
}

/// Extractor for routes that need a logged-in user. Anonymous requests are
/// redirected to `/login` and sent back afterwards. Works in any router whose
/// state implements `FromRef<S> for AuthState`.
pub struct RequireUser(pub User);

impl<S> FromRequestParts<S> for RequireUser
where
    AuthState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Redirect;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let auth = AuthState::from_ref(state);
        match auth.current_user(&parts.headers).await {
            Some(user) => Ok(RequireUser(user)),
            None => Err(login_redirect(
                parts
                    .uri
                    .path_and_query()
                    .map(|p| p.as_str())
                    .unwrap_or("/"),
            )),
        }
    }
}

/// Router with all login, setup and OAuth endpoints.
const EXEMPT_EXACT: [&str; 8] = [
    "/healthz",
    "/readyz",
    "/login",
    "/logout",
    "/auth/theme.css",
    "/api/openapi.json",
    "/api/docs",
    "/mcp",
];
const EXEMPT_PREFIX: [&str; 5] = [
    "/static/",
    "/oauth/",
    "/.well-known/",
    "/mcp/",
    "/auth/oidc/",
];

fn is_exempt(path: &str) -> bool {
    EXEMPT_EXACT.contains(&path) || EXEMPT_PREFIX.iter().any(|p| path.starts_with(p))
}

/// Requires a session (web UI) or an `/api`-scoped bearer token (API) on every
/// route except the exemptions above. `/mcp` is guarded by its own bearer layer.
/// Web requests are redirected to `/login`, API requests get a 401.
pub async fn enforce_auth(State(auth): State<AuthState>, request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    if !auth.required || is_exempt(&path) {
        return next.run(request).await;
    }
    if path == "/setup" && auth.store.count_users().await.unwrap_or(1) == 0 {
        return next.run(request).await;
    }
    let is_api = path == "/api" || path.starts_with("/api/");
    if auth.current_user(request.headers()).await.is_some() {
        return next.run(request).await;
    }
    if is_api {
        let validator = auth.validator("/api");
        if validator.validate(request.headers()).await.is_some() {
            return next.run(request).await;
        }
        let mut response = validator.unauthorized(request.headers());
        *response.body_mut() = axum::body::Body::from(r#"{"error":"unauthorized"}"#);
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        return response;
    }
    let next_path = if request.method() == axum::http::Method::GET {
        request
            .uri()
            .path_and_query()
            .map(|p| p.as_str().to_string())
            .unwrap_or_else(|| "/".into())
    } else {
        "/".into()
    };
    login_redirect(&next_path).into_response()
}

pub fn router<S>(state: AuthState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/auth/theme.css", get(theme))
        .route("/setup", get(setup_page).post(setup_submit))
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", post(logout))
        .route("/.well-known/oauth-authorization-server", get(as_metadata))
        .route(
            "/.well-known/oauth-protected-resource",
            get(resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(resource_metadata),
        )
        .route("/oauth/register", post(register))
        .route(
            "/oauth/authorize",
            get(authorize_page).post(authorize_submit),
        )
        .route("/oauth/token", post(token))
        .route("/oauth/revoke", post(revoke))
        .with_state(state.clone())
        .merge(super::oidc::router(state))
}

// ---- cookies and CSRF -------------------------------------------------------

pub(crate) fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

pub(crate) fn set_cookie(name: &str, value: &str, max_age: i64, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
}

/// Double-submit CSRF: the form field must equal the cookie. Returns the
/// token to embed plus a cookie to set when none exists yet.
fn csrf_token(headers: &HeaderMap, secure: bool) -> (String, Option<String>) {
    match cookie(headers, CSRF_COOKIE).filter(|c| c.len() >= 32) {
        Some(existing) => (existing, None),
        None => {
            let token = random_token();
            let set = set_cookie(CSRF_COOKIE, &token, SESSION_TTL_SECS, secure);
            (token, Some(set))
        }
    }
}

fn csrf_ok(headers: &HeaderMap, submitted: &str) -> bool {
    cookie(headers, CSRF_COOKIE).is_some_and(|c| c.len() >= 32 && ct_eq(&c, submitted))
}

fn with_cookies(mut response: Response, cookies: impl IntoIterator<Item = String>) -> Response {
    for c in cookies {
        if let Ok(v) = HeaderValue::from_str(&c) {
            response.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    response
}

pub(crate) fn render<T: Template>(template: T, status: StatusCode) -> Response {
    match template.render() {
        Ok(body) => (status, Html(body)).into_response(),
        Err(e) => {
            tracing::error!("template error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub(crate) fn login_redirect(next: &str) -> Redirect {
    let encoded: String = form_urlencoded::byte_serialize(next.as_bytes()).collect();
    Redirect::to(&format!("/login?next={encoded}"))
}

async fn start_session(auth: &AuthState, user: &User, secure: bool) -> Option<String> {
    let id = create_session(auth, user).await?;
    Some(set_cookie(SESSION_COOKIE, &id, SESSION_TTL_SECS, secure))
}

/// Stores a new session for the user and returns its cookie value.
pub(crate) async fn create_session(auth: &AuthState, user: &User) -> Option<String> {
    let id = random_token();
    auth.store
        .create_session(Session {
            id_hash: hash_token(&id),
            user_id: user.id.clone(),
            expires_at: now() + SESSION_TTL_SECS,
        })
        .await
        .map_err(|e| tracing::error!("session create failed: {e}"))
        .ok()?;
    Some(id)
}

pub(crate) fn is_secure(base: &str) -> bool {
    base.starts_with("https://")
}

// ---- templates --------------------------------------------------------------

#[derive(Template)]
#[template(path = "auth/setup.html")]
struct SetupTemplate {
    csrf: String,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "auth/login.html")]
struct LoginTemplate {
    csrf: String,
    next: String,
    error: Option<String>,
    sso: Option<SsoButton>,
    local_login: bool,
}

struct SsoButton {
    label: String,
    href: String,
}

impl AuthState {
    fn login_template(&self, csrf: String, next: String, error: Option<String>) -> LoginTemplate {
        let sso = self.oidc.as_ref().map(|oidc| {
            let encoded: String = form_urlencoded::byte_serialize(next.as_bytes()).collect();
            SsoButton {
                label: oidc.config.display_name.clone(),
                href: format!("/auth/oidc/login?next={encoded}"),
            }
        });
        LoginTemplate {
            csrf,
            next,
            error,
            sso,
            local_login: self.local_login_allowed(),
        }
    }

    pub(crate) fn local_login_allowed(&self) -> bool {
        self.oidc
            .as_ref()
            .is_none_or(|o| o.config.allow_local_login)
    }
}

#[derive(Template)]
#[template(path = "auth/consent.html")]
struct ConsentTemplate {
    csrf: String,
    headline: String,
    username: String,
    redirect_host: String,
    scopes: Vec<String>,
    fields: Vec<(&'static str, String)>,
}

#[derive(Template)]
#[template(path = "auth/error.html")]
struct ErrorTemplate {
    message: String,
}

async fn theme() -> impl IntoResponse {
    let css = concat!(
        include_str!("../../../../design/components.css"),
        "\n",
        include_str!("auth_theme.css")
    );
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], css)
}

// ---- setup / login / logout -------------------------------------------------

async fn setup_page(State(auth): State<AuthState>, headers: HeaderMap) -> Response {
    if auth.store.count_users().await.unwrap_or(1) > 0 || !auth.local_login_allowed() {
        return Redirect::to("/login").into_response();
    }
    let (csrf, cookie) = csrf_token(&headers, is_secure(&auth.base(&headers)));
    with_cookies(
        render(SetupTemplate { csrf, error: None }, StatusCode::OK),
        cookie,
    )
}

#[derive(Deserialize)]
struct SetupForm {
    username: String,
    password: String,
    csrf: String,
}

async fn setup_submit(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Form(form): Form<SetupForm>,
) -> Response {
    let secure = is_secure(&auth.base(&headers));
    if !csrf_ok(&headers, &form.csrf) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let form_error = |msg: &str| {
        render(
            SetupTemplate {
                csrf: form.csrf.clone(),
                error: Some(msg.to_string()),
            },
            StatusCode::UNPROCESSABLE_ENTITY,
        )
    };
    let username = form.username.trim();
    if username.is_empty() || username.len() > 64 {
        return form_error("Choose a username of up to 64 characters.");
    }
    if form.password.chars().count() < MIN_PASSWORD_LEN {
        return form_error("Use a password of at least 12 characters.");
    }
    let Ok(password_hash) = hash_password(&form.password) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let user = User {
        id: uuid::Uuid::new_v4().to_string(),
        username: username.to_string(),
        password_hash,
        created_at: now(),
        email: None,
        display_name: None,
        is_admin: true,
        auth_source: SOURCE_LOCAL.to_string(),
        external_id: None,
    };
    match auth.store.create_first_user(user).await {
        Ok(Some(user)) => match start_session(&auth, &user, secure).await {
            Some(session) => with_cookies(Redirect::to("/").into_response(), [session]),
            None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        Ok(None) => Redirect::to("/login").into_response(),
        Err(e) => {
            tracing::error!("setup failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
struct NextQuery {
    next: Option<String>,
}

async fn login_page(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Query(q): Query<NextQuery>,
) -> Response {
    if auth.store.count_users().await.unwrap_or(1) == 0 && auth.local_login_allowed() {
        return Redirect::to("/setup").into_response();
    }
    let next = safe_next(q.next.as_deref());
    if auth.current_user(&headers).await.is_some() {
        return Redirect::to(&next).into_response();
    }
    let (csrf, cookie) = csrf_token(&headers, is_secure(&auth.base(&headers)));
    with_cookies(
        render(auth.login_template(csrf, next, None), StatusCode::OK),
        cookie,
    )
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
    csrf: String,
    next: Option<String>,
}

async fn login_submit(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let secure = is_secure(&auth.base(&headers));
    if !csrf_ok(&headers, &form.csrf) || !auth.local_login_allowed() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let next = safe_next(form.next.as_deref());
    let user = auth
        .store
        .find_user_by_username(form.username.trim())
        .await
        .ok()
        .flatten();
    // Hash against a throwaway when the user is unknown so timing does not
    // reveal which usernames exist.
    let verified = match &user {
        Some(u) => verify_password(&form.password, &u.password_hash),
        None => {
            let _ = hash_password(&form.password);
            false
        }
    };
    match user.filter(|_| verified) {
        Some(user) => match start_session(&auth, &user, secure).await {
            Some(session) => with_cookies(Redirect::to(&next).into_response(), [session]),
            None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        None => render(
            auth.login_template(
                form.csrf,
                next,
                Some("That username and password don't match.".into()),
            ),
            StatusCode::UNAUTHORIZED,
        ),
    }
}

#[derive(Deserialize)]
struct CsrfForm {
    csrf: String,
}

async fn logout(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    if !csrf_ok(&headers, &form.csrf) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let signed_in_via_oidc = match (auth.current_user(&headers).await, &auth.oidc) {
        (Some(user), Some(_)) => user.auth_source.starts_with("oidc:"),
        _ => false,
    };
    if let Some(id) = cookie(&headers, SESSION_COOKIE) {
        let _ = auth.store.delete_session(&hash_token(&id)).await;
    }
    let base = auth.base(&headers);
    let clear = set_cookie(SESSION_COOKIE, "", 0, is_secure(&base));
    // End the provider session too when it supports RP-initiated logout.
    let end_session = match (&auth.oidc, signed_in_via_oidc) {
        (Some(oidc), true) => oidc.end_session_url(&format!("{base}/login")).await,
        _ => None,
    };
    let target = end_session.as_deref().unwrap_or("/login");
    with_cookies(Redirect::to(target).into_response(), [clear])
}

// ---- metadata and registration ----------------------------------------------

async fn as_metadata(State(auth): State<AuthState>, headers: HeaderMap) -> Json<Value> {
    Json(authorization_server_metadata(&auth.base(&headers)))
}

async fn resource_metadata(State(auth): State<AuthState>, headers: HeaderMap) -> Json<Value> {
    Json(protected_resource_metadata(&auth.base(&headers)))
}

fn oauth_json(status: StatusCode, body: Value) -> Response {
    let mut response = (status, Json(body)).into_response();
    let h = response.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        HeaderName::from_static("pragma"),
        HeaderValue::from_static("no-cache"),
    );
    response
}

fn oauth_error(e: OAuthError) -> Response {
    let status = match e.error {
        "invalid_client" => StatusCode::UNAUTHORIZED,
        "server_error" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    oauth_json(status, e.to_json())
}

async fn register(State(auth): State<AuthState>, Json(req): Json<RegistrationRequest>) -> Response {
    match auth.server.register_client(req).await {
        Ok(body) => oauth_json(StatusCode::CREATED, body),
        Err(e) => oauth_error(e),
    }
}

// ---- authorize --------------------------------------------------------------

pub(crate) fn error_page(message: &str) -> Response {
    error_page_with(message, StatusCode::BAD_REQUEST)
}

pub(crate) fn error_page_with(message: &str, status: StatusCode) -> Response {
    render(
        ErrorTemplate {
            message: message.to_string(),
        },
        status,
    )
}

fn redirect_with(redirect_uri: &str, params: &[(&str, &str)]) -> Response {
    match Url::parse(redirect_uri) {
        Ok(mut url) => {
            url.query_pairs_mut().extend_pairs(params);
            Redirect::to(url.as_str()).into_response()
        }
        Err(_) => error_page("The redirect address is not valid."),
    }
}

fn failure_response(f: AuthorizeFailure) -> Response {
    match f {
        AuthorizeFailure::Page(m) => error_page(&m),
        AuthorizeFailure::Redirect {
            redirect_uri,
            error,
            description,
            state,
        } => {
            let mut params = vec![
                ("error", error),
                ("error_description", description.as_str()),
            ];
            if let Some(s) = state.as_deref() {
                params.push(("state", s));
            }
            redirect_with(&redirect_uri, &params)
        }
    }
}

fn describe_scope(scope: &str) -> String {
    match scope {
        "documents:read" => "Read your documents".into(),
        "documents:write" => "Change your documents".into(),
        other => other.to_string(),
    }
}

fn headline(client: &str, scopes: &[String]) -> String {
    let read = scopes.iter().any(|s| s == "documents:read");
    let write = scopes.iter().any(|s| s == "documents:write");
    let what = match (read, write) {
        (true, true) => "read and change your documents",
        (false, true) => "change your documents",
        _ => "read your documents",
    };
    format!("Allow {client} to {what}?")
}

fn redirect_host(uri: &str) -> String {
    Url::parse(uri)
        .ok()
        .map(|u| match u.host_str() {
            Some(h) => h.to_string(),
            None => format!("{} app", u.scheme()),
        })
        .unwrap_or_else(|| "an unknown address".into())
}

async fn authorize_page(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    Query(params): Query<AuthorizeParams>,
) -> Response {
    let base = auth.base(&headers);
    let validated = match auth.server.validate_authorize(&base, &params).await {
        Ok(v) => v,
        Err(f) => return failure_response(f),
    };
    let Some(user) = auth.current_user(&headers).await else {
        let next = format!("/oauth/authorize?{}", raw.unwrap_or_default());
        return login_redirect(&next).into_response();
    };
    let (csrf, cookie) = csrf_token(&headers, is_secure(&base));
    let fields = vec![
        ("response_type", "code".to_string()),
        ("client_id", validated.client.client_id.clone()),
        ("redirect_uri", validated.redirect_uri.clone()),
        ("code_challenge", validated.code_challenge.clone()),
        ("code_challenge_method", "S256".to_string()),
        ("state", validated.state.clone().unwrap_or_default()),
        ("scope", validated.scopes.join(" ")),
        ("resource", validated.audience.clone()),
    ];
    let page = ConsentTemplate {
        csrf,
        headline: headline(&validated.client.client_name, &validated.scopes),
        username: user.username,
        redirect_host: redirect_host(&validated.redirect_uri),
        scopes: validated.scopes.iter().map(|s| describe_scope(s)).collect(),
        fields,
    };
    with_cookies(render(page, StatusCode::OK), cookie)
}

#[derive(Deserialize)]
struct ConsentForm {
    csrf: String,
    decision: String,
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    state: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
}

async fn authorize_submit(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Form(form): Form<ConsentForm>,
) -> Response {
    if !csrf_ok(&headers, &form.csrf) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(user) = auth.current_user(&headers).await else {
        return login_redirect("/").into_response();
    };
    let params = AuthorizeParams {
        response_type: form.response_type,
        client_id: form.client_id,
        redirect_uri: form.redirect_uri,
        code_challenge: form.code_challenge,
        code_challenge_method: form.code_challenge_method,
        state: form.state.filter(|s| !s.is_empty()),
        scope: form.scope,
        resource: form.resource,
    };
    let base = auth.base(&headers);
    let validated: ValidatedAuthorize = match auth.server.validate_authorize(&base, &params).await {
        Ok(v) => v,
        Err(f) => return failure_response(f),
    };
    let state = validated.state.as_deref();
    if form.decision != "allow" {
        let mut p = vec![("error", "access_denied")];
        if let Some(s) = state {
            p.push(("state", s));
        }
        return redirect_with(&validated.redirect_uri, &p);
    }
    match auth.server.issue_code(&user.id, &validated).await {
        Ok(code) => {
            let mut p = vec![("code", code.as_str())];
            if let Some(s) = state {
                p.push(("state", s));
            }
            redirect_with(&validated.redirect_uri, &p)
        }
        Err(e) => {
            tracing::error!("issue code failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---- token and revoke -------------------------------------------------------

async fn token(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Form(req): Form<TokenRequest>,
) -> Response {
    match auth.server.token(&auth.base(&headers), req).await {
        Ok(body) => oauth_json(StatusCode::OK, body),
        Err(e) => oauth_error(e),
    }
}

#[derive(Deserialize)]
struct RevokeForm {
    token: String,
}

async fn revoke(State(auth): State<AuthState>, Form(form): Form<RevokeForm>) -> Response {
    match auth.server.revoke(&form.token).await {
        Ok(()) => oauth_json(StatusCode::OK, serde_json::json!({})),
        Err(e) => oauth_error(e.into()),
    }
}
