//! Login, setup and the OAuth 2.1 authorization server endpoints.

use std::{sync::Arc, time::Duration};

use askama::Template;
use axum::{
    extract::{
        ConnectInfo, DefaultBodyLimit, FromRef, FromRequestParts, OriginalUri, Query, RawQuery,
        Request, State,
    },
    http::{header, request::Parts, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Semaphore;
use url::{form_urlencoded, Url};

use super::view::{SidebarUser, CURRENT_USER};
use super::{mfa, oidc, pending::Pending};

use folio_auth::{
    base_url,
    crypto::{
        ct_eq, hash_password, hash_token, hmac_token, random_secret, random_token, verify_password,
    },
    limit::{FailureLimiter, RateLimiter},
    model::SOURCE_LOCAL,
    oidc::OidcProvider,
    passkey::{build_webauthn, DiscoverableAuthentication, PasskeyRegistration, Webauthn},
    redirect::safe_next,
    secret::SecretBox,
    server::{
        authorization_server_metadata, now, protected_resource_metadata, AuthorizeFailure,
        AuthorizeParams, OAuthError, OAuthServer, RegistrationRequest, TokenRequest,
        ValidatedAuthorize,
    },
    AuthStore, ForwardAuth, Group, Session, TokenValidator, User,
};

// `__Host-` pins a cookie to this exact host over HTTPS, so a sibling
// subdomain cannot plant or overwrite it.
const SESSION_COOKIE: &str = "folio_session";
const CSRF_COOKIE: &str = "folio_csrf";
pub(crate) const SESSION_TTL_SECS: i64 = 14 * 24 * 3600;
const MIN_PASSWORD_LEN: usize = 12;
/// Dynamic client registrations a peer may make in a burst; the allowance
/// refills over an hour.
const REGISTER_BURST: u32 = 20;
const MAX_PASSWORD_LEN: usize = 1024;
const MAX_CONCURRENT_HASHES: usize = 4;
/// Failed logins tolerated before lockout. Per IP is looser because several
/// people can share an address.
const LOGIN_FAILURES_PER_USER: u32 = 5;
const LOGIN_FAILURES_PER_IP: u32 = 10;
const REGISTER_BODY_LIMIT: usize = 16 * 1024;
const OIDC_STARTS_PER_MINUTE: u32 = 30;
const PASSKEY_STARTS_PER_MINUTE: u32 = 30;
/// Wrong second-factor codes tolerated per user (and per address) before lockout.
const CODE_FAILURES_PER_USER: u32 = 5;
const CODE_FAILURES_PER_IP: u32 = 10;
/// Time to type the code after the password was accepted.
const MFA_LOGIN_TTL: Duration = Duration::from_secs(300);
const PASSKEY_CEREMONY_TTL: Duration = Duration::from_secs(300);

/// A password-verified login that still needs its second factor.
pub(crate) struct MfaLogin {
    pub user_id: String,
    pub next: String,
    pub attempts: u32,
}

/// A passkey login ceremony in flight. `mfa_user` is set when it answers a
/// pending second-factor step, and then only that user may complete it.
pub(crate) struct PasskeyAuth {
    pub state: DiscoverableAuthentication,
    pub mfa_user: Option<String>,
}

pub(crate) fn login_limiter(free_attempts: u32) -> FailureLimiter {
    FailureLimiter::new(
        free_attempts,
        Duration::from_secs(30),
        Duration::from_secs(15 * 60),
        Duration::from_secs(3600),
    )
}

/// State for the auth routes and the [`RequireUser`] extractor.
#[derive(Clone)]
pub struct AuthState {
    pub store: Arc<dyn AuthStore>,
    pub server: OAuthServer,
    pub public_url: Option<String>,
    /// When false, [`enforce_auth`] lets every request through.
    pub required: bool,
    /// Trusted-header login, see [`crate::forward::forward_auth`].
    pub forward: Option<Arc<ForwardAuth>>,
    /// Sign-in through an OpenID Connect provider.
    pub oidc: Option<Arc<OidcProvider>>,
    /// Caps how often one address can start an OIDC login; every start
    /// stores a row.
    oidc_starts: Arc<RateLimiter>,
    /// Seals TOTP seeds and keys recovery-code hashes.
    pub(crate) secret_box: Arc<SecretBox>,
    /// `None` when the public URL cannot be a WebAuthn relying party.
    pub(crate) webauthn: Option<Arc<Webauthn>>,
    pub(crate) mfa_logins: Arc<Pending<MfaLogin>>,
    pub(crate) passkey_auths: Arc<Pending<PasskeyAuth>>,
    /// Registration ceremonies, keyed by user id.
    pub(crate) passkey_regs: Arc<Pending<PasskeyRegistration>>,
    pub(crate) passkey_starts: Arc<RateLimiter>,
    pub(crate) code_by_ip: Arc<FailureLimiter>,
    pub(crate) code_by_user: Arc<FailureLimiter>,
    setup_token: Arc<str>,
    csrf_secret: Arc<[u8; 32]>,
    register_limit: Arc<RateLimiter>,
    login_by_ip: Arc<FailureLimiter>,
    pub(crate) login_by_user: Arc<FailureLimiter>,
    /// Bounds concurrent argon2 hashes, which are memory-hungry.
    hash_permits: Arc<Semaphore>,
}

impl AuthState {
    pub fn new(store: Arc<dyn AuthStore>, public_url: Option<String>) -> Self {
        Self {
            server: OAuthServer::new(store.clone()),
            store,
            public_url: public_url.clone(),
            required: true,
            forward: None,
            oidc: None,
            secret_box: Arc::new(
                SecretBox::new(&SecretBox::generate_key()).expect("generated keys are long enough"),
            ),
            webauthn: build_webauthn(&base_url(public_url.as_deref()))
                .map_err(|e| tracing::warn!("passkeys are unavailable: {e}"))
                .ok()
                .map(Arc::new),
            mfa_logins: Arc::new(Pending::new(MFA_LOGIN_TTL)),
            passkey_auths: Arc::new(Pending::new(PASSKEY_CEREMONY_TTL)),
            passkey_regs: Arc::new(Pending::new(PASSKEY_CEREMONY_TTL)),
            passkey_starts: Arc::new(RateLimiter::new(
                PASSKEY_STARTS_PER_MINUTE,
                PASSKEY_STARTS_PER_MINUTE as f64 / 60.0,
            )),
            code_by_ip: Arc::new(login_limiter(CODE_FAILURES_PER_IP)),
            code_by_user: Arc::new(login_limiter(CODE_FAILURES_PER_USER)),
            oidc_starts: Arc::new(RateLimiter::new(
                OIDC_STARTS_PER_MINUTE,
                OIDC_STARTS_PER_MINUTE as f64 / 60.0,
            )),
            setup_token: random_token().into(),
            csrf_secret: Arc::new(random_secret()),
            login_by_ip: Arc::new(login_limiter(LOGIN_FAILURES_PER_IP)),
            login_by_user: Arc::new(login_limiter(LOGIN_FAILURES_PER_USER)),
            hash_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_HASHES)),
            register_limit: Arc::new(RateLimiter::new(
                REGISTER_BURST,
                REGISTER_BURST as f64 / 3600.0,
            )),
        }
    }

    /// Replaces the generated one-time setup token (for example with an
    /// operator-chosen value from the configuration).
    pub fn with_setup_token(mut self, token: impl Into<Arc<str>>) -> Self {
        self.setup_token = token.into();
        self
    }

    /// Starts the periodic cleanup of expired tokens and unused clients.
    pub fn spawn_maintenance(&self) -> tokio::task::JoinHandle<()> {
        let server = self.server.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(600));
            loop {
                tick.tick().await;
                if let Err(e) = server.purge().await {
                    tracing::warn!("auth cleanup failed: {e}");
                }
            }
        })
    }

    /// Secret that must accompany `/setup` while no user exists.
    pub fn setup_token(&self) -> &str {
        &self.setup_token
    }

    fn setup_token_ok(&self, given: Option<&str>) -> bool {
        given.is_some_and(|g| !g.is_empty() && ct_eq(g, &self.setup_token))
    }

    /// Sets the key that encrypts TOTP seeds at rest. Without it a random key
    /// is used, which does not survive a restart.
    pub fn with_secret_key(mut self, key: &str) -> Result<Self, String> {
        self.secret_box = Arc::new(SecretBox::new(key)?);
        Ok(self)
    }

    /// Whether passkeys can be offered.
    pub(crate) fn passkeys_enabled(&self) -> bool {
        self.webauthn.is_some()
    }

    pub fn with_forward(mut self, forward: ForwardAuth) -> Self {
        self.forward = Some(Arc::new(forward));
        self
    }

    pub fn with_oidc(mut self, oidc: OidcProvider) -> Self {
        self.oidc = Some(Arc::new(oidc));
        self
    }

    /// Whether the username and password form works.
    pub(crate) fn local_login_allowed(&self) -> bool {
        self.oidc.as_ref().is_none_or(|o| o.allow_local_login())
    }

    /// `Err(retry_after_secs)` when `ip` started too many OIDC logins lately.
    pub(crate) fn oidc_start_allowed(&self, ip: &str) -> Result<(), u64> {
        self.oidc_starts.check(ip)
    }

    pub fn with_required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// The user's groups in both namespaces, resolved on demand so callers
    /// (session or token holders alike) always see current memberships.
    pub async fn effective_groups(&self, user_id: &str) -> Vec<Group> {
        self.store.user_groups(user_id).await.unwrap_or_else(|e| {
            tracing::error!("group lookup failed: {e}");
            Vec::new()
        })
    }

    /// Validator for a protected resource path such as `/mcp`.
    pub fn validator(&self, resource_path: &str) -> TokenValidator {
        TokenValidator::new(self.store.clone(), self.public_url.clone(), resource_path)
    }

    /// Runs a password hash on the blocking pool, at most
    /// [`MAX_CONCURRENT_HASHES`] at a time.
    pub(crate) async fn hash_work<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        let _permit = self.hash_permits.acquire().await.ok()?;
        tokio::task::spawn_blocking(work).await.ok()
    }

    /// Cookie-authenticated writes must come from our own pages. Fetch metadata
    /// is authoritative when the browser sends it; `Origin` covers older ones.
    /// Requests with neither are not from a browser and carry no ambient cookie.
    pub(crate) fn is_same_origin(&self, headers: &HeaderMap) -> bool {
        let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        if let Some(site) = text("sec-fetch-site") {
            return matches!(site, "same-origin" | "none");
        }
        match text("origin") {
            None => true,
            Some(origin) => Url::parse(&self.base())
                .is_ok_and(|base| base.origin().ascii_serialization() == origin),
        }
    }

    pub(crate) fn base(&self) -> String {
        base_url(self.public_url.as_deref())
    }

    pub(crate) fn secure(&self) -> bool {
        self.base().starts_with("https://")
    }

    pub(crate) fn session_cookie_name(&self) -> &'static str {
        if self.secure() {
            "__Host-folio_session"
        } else {
            SESSION_COOKIE
        }
    }

    pub(crate) fn csrf_cookie_name(&self) -> &'static str {
        if self.secure() {
            "__Host-folio_csrf"
        } else {
            CSRF_COOKIE
        }
    }

    /// The raw session cookie value and its user, when the session is live.
    pub(crate) async fn session(&self, headers: &HeaderMap) -> Option<(String, User)> {
        let id = cookie(headers, self.session_cookie_name())?;
        let session = self.store.find_session(&hash_token(&id)).await.ok()??;
        if session.expires_at <= now() {
            return None;
        }
        let user = self.store.find_user_by_id(&session.user_id).await.ok()??;
        Some((id, user))
    }

    pub(crate) async fn current_user(&self, headers: &HeaderMap) -> Option<User> {
        self.session(headers).await.map(|(_, user)| user)
    }

    fn csrf_mac(&self, subject: &str) -> String {
        hmac_token(&*self.csrf_secret, subject)
    }

    /// CSRF token for a form shown before login (login, setup), plus a cookie
    /// to set when needed. It is an HMAC, under a server secret, of a random
    /// pre-session cookie, so it cannot be computed without asking the server.
    pub(crate) fn csrf_pre_token(&self, headers: &HeaderMap) -> (String, Option<String>) {
        let (value, set) = match cookie(headers, self.csrf_cookie_name()).filter(|c| c.len() >= 32)
        {
            Some(existing) => (existing, None),
            None => {
                let value = random_token();
                let set = set_cookie(
                    self.csrf_cookie_name(),
                    &value,
                    SESSION_TTL_SECS,
                    self.secure(),
                );
                (value, Some(set))
            }
        };
        (self.csrf_mac(&format!("pre:{value}")), set)
    }

    pub(crate) fn csrf_pre_ok(&self, headers: &HeaderMap, submitted: &str) -> bool {
        cookie(headers, self.csrf_cookie_name())
            .filter(|c| c.len() >= 32)
            .is_some_and(|c| ct_eq(&self.csrf_mac(&format!("pre:{c}")), submitted))
    }

    /// CSRF token for a form shown to a logged-in user: an HMAC of the session
    /// id, so a token from one session is worthless in another.
    pub fn csrf_session_token(&self, session_id: &str) -> String {
        self.csrf_mac(&format!("session:{session_id}"))
    }

    pub(crate) async fn csrf_session_ok(&self, headers: &HeaderMap, submitted: &str) -> bool {
        match self.session(headers).await {
            Some((id, _)) => ct_eq(&self.csrf_session_token(&id), submitted),
            None => false,
        }
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
const EXEMPT_EXACT: [&str; 11] = [
    "/healthz",
    "/readyz",
    "/login",
    "/login/code",
    "/logout",
    "/auth/theme.css",
    "/api/openapi.json",
    "/api/docs",
    "/mcp",
    "/manifest.webmanifest",
    "/sw.js",
];
const EXEMPT_PREFIX: [&str; 6] = [
    "/auth/passkey/",
    "/static/",
    "/oauth/",
    "/.well-known/",
    "/mcp/",
    "/auth/oidc/",
];

pub(crate) fn is_exempt(path: &str) -> bool {
    EXEMPT_EXACT.contains(&path) || EXEMPT_PREFIX.iter().any(|p| path.starts_with(p))
}

/// Requires a session (web UI) or an `/api`-scoped bearer token (API) on every
/// route except the exemptions above. `/mcp` is guarded by its own bearer layer.
/// Web requests are redirected to `/login`, API requests get a 401.
pub async fn enforce_auth(
    State(auth): State<AuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    if !auth.required || is_exempt(&path) {
        return next.run(request).await;
    }
    if path == "/setup" && auth.store.count_users().await.unwrap_or(1) == 0 {
        return next.run(request).await;
    }
    let is_api = path == "/api" || path.starts_with("/api/");
    if let Some((session_id, user)) = auth.session(request.headers()).await {
        if !is_read_method(request.method()) && !auth.is_same_origin(request.headers()) {
            return StatusCode::FORBIDDEN.into_response();
        }
        let sidebar_user = SidebarUser::new(&user, auth.csrf_session_token(&session_id));
        return CURRENT_USER.scope(sidebar_user, next.run(request)).await;
    }
    if is_api {
        let validator = auth.validator("/api");
        if let Some(context) = validator.validate(request.headers()).await {
            let needed = if is_read_method(request.method()) || path.starts_with("/api/telemetry/")
            {
                "documents:read"
            } else {
                "documents:write"
            };
            if !context.has_scope(needed) {
                return insufficient_scope(needed);
            }
            request.extensions_mut().insert(context);
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
    if path == "/share" && request.method() == Method::POST {
        // Accepting the upload before login would let anyone park files on
        // disk, so ask the user to sign in and share again instead.
        return render(ShareLoginTemplate, StatusCode::UNAUTHORIZED);
    }
    let next_path = if request.method() == Method::GET {
        request
            .extensions()
            .get::<OriginalUri>()
            .map(|original| &original.0)
            .unwrap_or_else(|| request.uri())
            .path_and_query()
            .map(|p| p.as_str().to_string())
            .unwrap_or_else(|| "/".into())
    } else {
        "/".into()
    };
    login_redirect(&next_path).into_response()
}

fn is_read_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn insufficient_scope(scope: &str) -> Response {
    let mut response = (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"insufficient_scope"}"#,
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&format!(
        "Bearer error=\"insufficient_scope\", scope=\"{scope}\""
    )) {
        response.headers_mut().insert(header::WWW_AUTHENTICATE, v);
    }
    response
}

pub fn router<S>(state: AuthState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/auth/theme.css", get(theme))
        .route("/setup", get(setup_page).post(setup_submit))
        .route("/login", get(login_page).post(login_submit))
        .route("/login/code", get(mfa::code_page).post(mfa::code_submit))
        .route("/auth/passkey/start", post(mfa::passkey_start))
        .route("/auth/passkey/finish", post(mfa::passkey_finish))
        .route("/logout", post(logout))
        .route("/auth/oidc/login", get(oidc::start))
        .route("/auth/oidc/callback", get(oidc::callback))
        .route("/.well-known/oauth-authorization-server", get(as_metadata))
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(mcp_resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/api",
            get(api_resource_metadata),
        )
        .route(
            "/oauth/register",
            post(register).layer(DefaultBodyLimit::max(REGISTER_BODY_LIMIT)),
        )
        .route(
            "/oauth/authorize",
            get(authorize_page).post(authorize_submit),
        )
        .route("/oauth/token", post(token))
        .route("/oauth/revoke", post(revoke))
        .with_state(state)
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

pub(crate) fn with_cookies(
    mut response: Response,
    cookies: impl IntoIterator<Item = String>,
) -> Response {
    for c in cookies {
        if let Ok(v) = HeaderValue::from_str(&c) {
            response.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    response
}

pub(crate) fn render<T: Template>(template: T, status: StatusCode) -> Response {
    match template.render() {
        Ok(body) => {
            let mut response = (status, Html(body)).into_response();
            let h = response.headers_mut();
            // Clickjacking and back-button exposure of credentials and consent.
            h.insert("x-frame-options", HeaderValue::from_static("DENY"));
            h.insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("frame-ancestors 'none'"),
            );
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(e) => {
            tracing::error!("template error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn login_redirect(next: &str) -> Redirect {
    let encoded: String = form_urlencoded::byte_serialize(next.as_bytes()).collect();
    Redirect::to(&format!("/login?next={encoded}"))
}

pub(crate) async fn start_session(auth: &AuthState, user: &User) -> Option<String> {
    let id = create_session(auth, user, SESSION_TTL_SECS).await?;
    Some(set_cookie(
        auth.session_cookie_name(),
        &id,
        SESSION_TTL_SECS,
        auth.secure(),
    ))
}

/// Stores a new session for the user and returns its cookie value.
pub(crate) async fn create_session(auth: &AuthState, user: &User, ttl: i64) -> Option<String> {
    let id = random_token();
    auth.store
        .create_session(Session {
            id_hash: hash_token(&id),
            user_id: user.id.clone(),
            expires_at: now() + ttl,
            created_at: now(),
            user_agent: None,
        })
        .await
        .map_err(|e| tracing::error!("session create failed: {e}"))
        .ok()?;
    Some(id)
}

// ---- templates --------------------------------------------------------------

#[derive(Template)]
#[template(path = "auth/setup.html")]
struct SetupTemplate {
    csrf: String,
    token: String,
    error: Option<String>,
    sso: Option<Sso>,
}

/// The single sign-on button.
struct Sso {
    label: String,
    href: String,
}

fn sso_button(auth: &AuthState, next: &str) -> Option<Sso> {
    let provider = auth.oidc.as_ref()?;
    let encoded: String = form_urlencoded::byte_serialize(next.as_bytes()).collect();
    Some(Sso {
        label: provider.display_name().to_string(),
        href: format!("/auth/oidc/login?next={encoded}"),
    })
}

#[derive(Template)]
#[template(path = "auth/setup_locked.html")]
struct SetupLockedTemplate;

#[derive(Template)]
#[template(path = "auth/share_login.html")]
struct ShareLoginTemplate;

#[derive(Template)]
#[template(path = "auth/login.html")]
struct LoginTemplate {
    csrf: String,
    next: String,
    error: Option<String>,
    sso: Option<Sso>,
    password_form: bool,
    passkeys: bool,
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
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("auth_theme.css"),
    )
}

// ---- setup / login / logout -------------------------------------------------

#[derive(Deserialize)]
struct SetupQuery {
    token: Option<String>,
}

async fn setup_page(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Query(q): Query<SetupQuery>,
) -> Response {
    if auth.store.count_users().await.unwrap_or(1) > 0 || !auth.local_login_allowed() {
        return Redirect::to("/login").into_response();
    }
    let Some(token) = q.token.filter(|t| auth.setup_token_ok(Some(t))) else {
        return render(SetupLockedTemplate, StatusCode::FORBIDDEN);
    };
    let (csrf, cookie) = auth.csrf_pre_token(&headers);
    with_cookies(
        render(
            SetupTemplate {
                csrf,
                token,
                error: None,
                // The table is empty, so signing in creates the owner.
                sso: sso_button(&auth, "/"),
            },
            StatusCode::OK,
        ),
        cookie,
    )
}

#[derive(Deserialize)]
struct SetupForm {
    username: String,
    password: String,
    csrf: String,
    token: Option<String>,
}

async fn setup_submit(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Form(form): Form<SetupForm>,
) -> Response {
    if auth.store.count_users().await.unwrap_or(1) > 0 {
        return Redirect::to("/login").into_response();
    }
    if !auth.local_login_allowed() || !auth.csrf_pre_ok(&headers, &form.csrf) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !auth.setup_token_ok(form.token.as_deref()) {
        return render(SetupLockedTemplate, StatusCode::FORBIDDEN);
    }
    let form_error = |msg: &str| {
        render(
            SetupTemplate {
                csrf: form.csrf.clone(),
                token: form.token.clone().unwrap_or_default(),
                error: Some(msg.to_string()),
                sso: sso_button(&auth, "/"),
            },
            StatusCode::UNPROCESSABLE_ENTITY,
        )
    };
    let username = form.username.trim();
    if username.is_empty() || username.len() > 64 {
        return form_error("Choose a username of up to 64 characters.");
    }
    let password_len = form.password.chars().count();
    if password_len < MIN_PASSWORD_LEN {
        return form_error("Use a password of at least 12 characters.");
    }
    if password_len > MAX_PASSWORD_LEN {
        return form_error("Use a password of at most 1024 characters.");
    }
    let password = form.password.clone();
    let Some(Ok(password_hash)) = auth.hash_work(move || hash_password(&password)).await else {
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
        is_owner: false,
        auth_source: SOURCE_LOCAL.to_string(),
        external_id: None,
    };
    match auth.store.create_first_user(user).await {
        Ok(Some(user)) => match start_session(&auth, &user).await {
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
pub(crate) struct NextQuery {
    pub(crate) next: Option<String>,
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
    let (csrf, cookie) = auth.csrf_pre_token(&headers);
    with_cookies(
        render(
            LoginTemplate {
                csrf,
                sso: sso_button(&auth, &next),
                password_form: auth.local_login_allowed(),
                passkeys: auth.passkeys_enabled() && auth.local_login_allowed(),
                next,
                error: None,
            },
            StatusCode::OK,
        ),
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
    ClientIp(ip): ClientIp,
    Form(form): Form<LoginForm>,
) -> Response {
    if !auth.local_login_allowed() || !auth.csrf_pre_ok(&headers, &form.csrf) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let next = safe_next(form.next.as_deref());
    let username = form.username.trim().to_lowercase();
    let retry_after = auth
        .login_by_ip
        .check(&ip)
        .err()
        .max(auth.login_by_user.check(&username).err());
    if let Some(seconds) = retry_after {
        let mut response = render(
            LoginTemplate {
                csrf: form.csrf,
                sso: sso_button(&auth, &next),
                password_form: true,
                passkeys: auth.passkeys_enabled(),
                next,
                error: Some(format!(
                    "Too many failed attempts. Try again in {}.",
                    wait_text(seconds)
                )),
            },
            StatusCode::TOO_MANY_REQUESTS,
        );
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        return response;
    }

    let user = auth
        .store
        .find_user_by_username(&username)
        .await
        .ok()
        .flatten();
    let verified = if form.password.chars().count() > MAX_PASSWORD_LEN {
        false
    } else {
        let password = form.password.clone();
        // Hash against a throwaway when the user is unknown so timing does not
        // reveal which usernames exist.
        let stored = user.as_ref().map(|u| u.password_hash.clone());
        auth.hash_work(move || match stored {
            Some(hash) => verify_password(&password, &hash),
            None => {
                let _ = hash_password(&password);
                false
            }
        })
        .await
        .unwrap_or(false)
    };
    match user.filter(|_| verified) {
        Some(user) => {
            auth.login_by_user.record_success(&username);
            if let Some(response) = mfa::challenge(&auth, &user, &next).await {
                return response;
            }
            // Fresh session id on login, so one planted beforehand is worthless.
            if let Some(id) = cookie(&headers, auth.session_cookie_name()) {
                let _ = auth.store.delete_session(&hash_token(&id)).await;
            }
            match start_session(&auth, &user).await {
                Some(session) => with_cookies(Redirect::to(&next).into_response(), [session]),
                None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        None => {
            auth.login_by_ip.record_failure(&ip);
            auth.login_by_user.record_failure(&username);
            render(
                LoginTemplate {
                    csrf: form.csrf,
                    sso: sso_button(&auth, &next),
                    password_form: true,
                    passkeys: auth.passkeys_enabled(),
                    next,
                    error: Some("That username and password don't match.".into()),
                },
                StatusCode::UNAUTHORIZED,
            )
        }
    }
}

pub(crate) fn wait_text(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds} seconds"),
        s => format!("{} minutes", s.div_ceil(60)),
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
    // Without a live session there is nothing to protect; just clear the cookie.
    let mut provider_logout = None;
    if let Some((id, user)) = auth.session(&headers).await {
        if !auth.csrf_session_ok(&headers, &form.csrf).await {
            return StatusCode::FORBIDDEN.into_response();
        }
        let hash = hash_token(&id);
        let id_token_hint = auth.store.session_id_token(&hash).await.ok().flatten();
        let _ = auth.store.delete_session(&hash).await;
        // Also end the session at the provider, or its single sign-on would
        // log the user straight back in.
        if let Some(p) = auth
            .oidc
            .as_ref()
            .filter(|p| user.auth_source == p.source())
        {
            provider_logout = p
                .logout_url(&format!("{}/login", auth.base()), id_token_hint.as_deref())
                .await;
        }
    }
    let clear = set_cookie(auth.session_cookie_name(), "", 0, auth.secure());
    let target = provider_logout.map_or_else(|| "/login".to_string(), |u| u.to_string());
    with_cookies(Redirect::to(&target).into_response(), [clear])
}

// ---- metadata and registration ----------------------------------------------

async fn as_metadata(State(auth): State<AuthState>) -> Json<Value> {
    Json(authorization_server_metadata(&auth.base()))
}

async fn mcp_resource_metadata(State(auth): State<AuthState>) -> Json<Value> {
    Json(protected_resource_metadata(&auth.base(), "/mcp"))
}

async fn api_resource_metadata(State(auth): State<AuthState>) -> Json<Value> {
    Json(protected_resource_metadata(&auth.base(), "/api"))
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

/// Peer address of the TCP connection. Proxy headers are deliberately ignored:
/// they are client-controlled unless a trusted proxy sets them.
pub(crate) struct ClientIp(pub(crate) String);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0.ip().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        Ok(Self(ip))
    }
}

fn too_many_requests(retry_after: u64) -> Response {
    let mut response = oauth_json(
        StatusCode::TOO_MANY_REQUESTS,
        serde_json::json!({"error": "too_many_requests", "error_description": "slow down"}),
    );
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from(retry_after));
    response
}

async fn register(
    State(auth): State<AuthState>,
    ClientIp(ip): ClientIp,
    Json(req): Json<RegistrationRequest>,
) -> Response {
    if let Err(retry_after) = auth.register_limit.check(&ip) {
        return too_many_requests(retry_after);
    }
    match auth.server.register_client(req).await {
        Ok(body) => oauth_json(StatusCode::CREATED, body),
        Err(e) => oauth_error(e),
    }
}

// ---- authorize --------------------------------------------------------------

fn error_page(message: &str) -> Response {
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

/// Redirects to the client with `params`, adding the RFC 9207 `iss` so a client
/// can tell which authorization server answered.
fn redirect_with(redirect_uri: &str, issuer: &str, params: &[(&str, &str)]) -> Response {
    match Url::parse(redirect_uri) {
        Ok(mut url) => {
            url.query_pairs_mut()
                .extend_pairs(params)
                .append_pair("iss", issuer);
            Redirect::to(url.as_str()).into_response()
        }
        Err(_) => error_page("The redirect address is not valid."),
    }
}

fn failure_response(f: AuthorizeFailure, issuer: &str) -> Response {
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
            redirect_with(&redirect_uri, issuer, &params)
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
    let base = auth.base();
    let validated = match auth.server.validate_authorize(&base, &params).await {
        Ok(v) => v,
        Err(f) => return failure_response(f, &base),
    };
    let Some((session_id, user)) = auth.session(&headers).await else {
        let next = format!("/oauth/authorize?{}", raw.unwrap_or_default());
        return login_redirect(&next).into_response();
    };
    let csrf = auth.csrf_session_token(&session_id);
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
    render(page, StatusCode::OK)
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
    if !auth.csrf_session_ok(&headers, &form.csrf).await {
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
    let base = auth.base();
    let validated: ValidatedAuthorize = match auth.server.validate_authorize(&base, &params).await {
        Ok(v) => v,
        Err(f) => return failure_response(f, &base),
    };
    let state = validated.state.as_deref();
    if form.decision != "allow" {
        let mut p = vec![("error", "access_denied")];
        if let Some(s) = state {
            p.push(("state", s));
        }
        return redirect_with(&validated.redirect_uri, &base, &p);
    }
    match auth.server.issue_code(&user.id, &validated).await {
        Ok(code) => {
            let mut p = vec![("code", code.as_str())];
            if let Some(s) = state {
                p.push(("state", s));
            }
            redirect_with(&validated.redirect_uri, &base, &p)
        }
        Err(e) => {
            tracing::error!("issue code failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

// ---- token and revoke -------------------------------------------------------

async fn token(State(auth): State<AuthState>, Form(req): Form<TokenRequest>) -> Response {
    match auth.server.token(&auth.base(), req).await {
        Ok(body) => oauth_json(StatusCode::OK, body),
        Err(e) => oauth_error(e),
    }
}

#[derive(Deserialize)]
struct RevokeForm {
    token: String,
    client_id: String,
}

async fn revoke(State(auth): State<AuthState>, Form(form): Form<RevokeForm>) -> Response {
    match auth.server.revoke(&form.token, &form.client_id).await {
        Ok(()) => oauth_json(StatusCode::OK, serde_json::json!({})),
        Err(e) => oauth_error(e.into()),
    }
}
