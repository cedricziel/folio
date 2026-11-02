//! End-to-end tests of the auth routes against a temp SQLite database.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    middleware,
    routing::get,
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use folio_api::{auth_router, AuthState};
use folio_auth::require_bearer;
use folio_infra::{create_pool, run_migrations, SqliteAuthStore};

const BASE: &str = "http://folio.test";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const REDIRECT: &str = "http://127.0.0.1:33418/callback";
const SETUP_TOKEN: &str = "test-setup-token";

struct App {
    router: Router,
    _dir: tempfile::TempDir,
}

async fn app() -> App {
    app_at(Some(BASE)).await
}

async fn app_at(public_url: Option<&str>) -> App {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("t.db").display());
    let pool = create_pool(&url).await.unwrap();
    run_migrations(&pool).await.unwrap();
    let auth = AuthState::new(
        Arc::new(SqliteAuthStore::new(pool)),
        public_url.map(String::from),
    )
    .with_setup_token(SETUP_TOKEN);
    let protected =
        Router::new()
            .route("/mcp", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                auth.validator("/mcp"),
                require_bearer,
            ));
    App {
        router: auth_router(auth).merge(protected),
        _dir: dir,
    }
}

struct Resp {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("not json: {}", self.body))
    }
    fn location(&self) -> String {
        self.headers[header::LOCATION].to_str().unwrap().to_string()
    }
    fn cookies(&self) -> Vec<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string())
            .collect()
    }
}

async fn send(app: &App, req: Request<Body>) -> Resp {
    let res = app.router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let body =
        String::from_utf8_lossy(&res.into_body().collect().await.unwrap().to_bytes()).to_string();
    Resp {
        status,
        headers,
        body,
    }
}

fn get_req(uri: &str, cookie: &str) -> Request<Body> {
    Request::get(uri)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap()
}

fn form_req(uri: &str, cookie: &str, body: &str) -> Request<Body> {
    Request::post(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, cookie)
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn challenge() -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(VERIFIER.as_bytes()))
}

/// Pulls the hidden csrf value out of a rendered form.
fn csrf_from(body: &str) -> String {
    let marker = "name=\"csrf\" value=\"";
    let start = body.find(marker).unwrap() + marker.len();
    body[start..start + body[start..].find('"').unwrap()].to_string()
}

/// The `name=value` pair of the CSRF cookie a page just set.
fn csrf_cookie(page: &Resp) -> String {
    page.cookies()
        .into_iter()
        .find(|c| c.contains("folio_csrf="))
        .expect("page sets a csrf cookie")
}

async fn setup_and_login(app: &App) -> String {
    let page = send(
        app,
        Request::get(format!("/setup?token={SETUP_TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    let csrf = csrf_from(&page.body);
    let cookie = csrf_cookie(&page);
    let done = send(
        app,
        form_req(
            "/setup",
            &cookie,
            &format!(
                "csrf={csrf}&token={SETUP_TOKEN}&username=owner&password=correct-horse-battery"
            ),
        ),
    )
    .await;
    assert_eq!(done.status, StatusCode::SEE_OTHER);
    let session = done
        .cookies()
        .into_iter()
        .find(|c| c.starts_with("folio_session="))
        .unwrap();
    format!("{cookie}; {session}")
}

async fn register(app: &App) -> String {
    let res = send(
        app,
        Request::post("/oauth/register")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"client_name": "Claude", "redirect_uris": [REDIRECT],
                       "token_endpoint_auth_method": "none"})
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    res.json()["client_id"].as_str().unwrap().to_string()
}

fn authorize_query(client_id: &str) -> String {
    format!(
        "response_type=code&client_id={client_id}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state=xyz&resource={}",
        enc(REDIRECT),
        challenge(),
        enc(&format!("{BASE}/mcp"))
    )
}

/// Runs consent and returns the authorization code.
async fn obtain_code(app: &App, session: &str, client_id: &str) -> String {
    let q = authorize_query(client_id);
    let page = send(app, get_req(&format!("/oauth/authorize?{q}"), session)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page
        .body
        .contains("Allow Claude to read and change your documents?"));
    assert!(page.body.contains("127.0.0.1"));
    let csrf = csrf_from(&page.body);
    let done = send(
        app,
        form_req(
            "/oauth/authorize",
            session,
            &format!("{q}&csrf={csrf}&decision=allow"),
        ),
    )
    .await;
    assert_eq!(done.status, StatusCode::SEE_OTHER);
    let loc = url::Url::parse(&done.location()).unwrap();
    assert!(loc.as_str().starts_with(REDIRECT));
    let pairs: std::collections::HashMap<_, _> = loc.query_pairs().into_owned().collect();
    assert_eq!(pairs["state"], "xyz");
    assert_eq!(pairs["iss"], BASE, "RFC 9207 issuer identification");
    pairs["code"].clone()
}

async fn exchange(app: &App, body: &str) -> Resp {
    send(app, form_req("/oauth/token", "", body)).await
}

fn code_body(client_id: &str, code: &str, verifier: &str) -> String {
    format!(
        "grant_type=authorization_code&client_id={client_id}&code={code}&code_verifier={verifier}&redirect_uri={}&resource={}",
        enc(REDIRECT),
        enc(&format!("{BASE}/mcp"))
    )
}

#[tokio::test]
async fn metadata_documents_have_expected_shape() {
    let app = app().await;
    let meta = send(
        &app,
        Request::get("/.well-known/oauth-authorization-server")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .json();
    assert_eq!(meta["issuer"], BASE);
    assert_eq!(meta["token_endpoint"], format!("{BASE}/oauth/token"));
    assert_eq!(
        meta["registration_endpoint"],
        format!("{BASE}/oauth/register")
    );
    assert_eq!(meta["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(meta["authorization_response_iss_parameter_supported"], true);
    assert_eq!(
        meta["token_endpoint_auth_methods_supported"],
        json!(["none"])
    );
    let prm = send(
        &app,
        Request::get("/.well-known/oauth-protected-resource/mcp")
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .json();
    assert_eq!(prm["resource"], format!("{BASE}/mcp"));
    assert_eq!(prm["authorization_servers"], json!([BASE]));
}

/// Requests as sent by a client that forges the headers a proxy would set.
fn spoofed(uri: &str) -> Request<Body> {
    Request::get(uri)
        .header(header::HOST, "evil.example")
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-host", "evil.example")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn urls_come_from_public_url_never_from_request_headers() {
    let app = app().await;
    let meta = send(&app, spoofed("/.well-known/oauth-authorization-server"))
        .await
        .json();
    assert_eq!(meta["issuer"], BASE);
    let prm = send(&app, spoofed("/.well-known/oauth-protected-resource/mcp"))
        .await
        .json();
    assert_eq!(prm["resource"], format!("{BASE}/mcp"));
    let challenge = send(&app, spoofed("/mcp")).await;
    let challenge = challenge.headers[header::WWW_AUTHENTICATE]
        .to_str()
        .unwrap();
    assert!(challenge.contains(BASE) && !challenge.contains("evil.example"));

    // Without a configured URL the fallback is fixed too, not taken from Host.
    let bare = app_at(None).await;
    let meta = send(&bare, spoofed("/.well-known/oauth-authorization-server"))
        .await
        .json();
    assert!(!meta["issuer"].as_str().unwrap().contains("evil.example"));
    assert!(meta["issuer"].as_str().unwrap().starts_with("http://"));
}

#[tokio::test]
async fn cookies_are_secure_only_when_public_url_is_https() {
    let session_cookie = |headers: &axum::http::HeaderMap| {
        headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .find(|c| c.contains("folio_csrf"))
            .unwrap()
    };
    let plain = app().await;
    let login = send(&plain, spoofed("/setup?token=test-setup-token")).await;
    assert!(
        !session_cookie(&login.headers).contains("Secure"),
        "X-Forwarded-Proto must not turn Secure on"
    );

    let tls = app_at(Some("https://folio.example.com")).await;
    let setup = send(
        &tls,
        Request::get("/setup?token=test-setup-token")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(session_cookie(&setup.headers).contains("Secure"));
}

#[tokio::test]
async fn registration_accepts_valid_and_rejects_bad_redirects() {
    let app = app().await;
    let post = |body: Value| {
        Request::post("/oauth/register")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    for ok in [
        "https://claude.ai/cb",
        "http://localhost:1234/cb",
        "http://127.0.0.1/cb",
        "com.example.app:/cb",
    ] {
        let r = send(&app, post(json!({"redirect_uris": [ok]}))).await;
        assert_eq!(r.status, StatusCode::CREATED, "{ok}: {}", r.body);
    }
    for bad in [
        "http://evil.example/cb",
        "javascript:alert(1)",
        "https://a.example/cb#x",
        "nope",
    ] {
        let r = send(&app, post(json!({"redirect_uris": [bad]}))).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(r.json()["error"], "invalid_redirect_uri");
    }
    let r = send(&app, post(json!({"redirect_uris": []}))).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = send(&app, post(json!({"redirect_uris": ["https://a.example/cb"], "token_endpoint_auth_method": "client_secret_basic"}))).await;
    assert_eq!(r.json()["error"], "invalid_client_metadata");
}

fn register_req(body: String) -> Request<Body> {
    Request::post("/oauth/register")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn registration_bounds_the_size_of_its_input() {
    let app = app().await;
    let uri = |n: usize| format!("https://a.example/{}", "x".repeat(n));

    let too_big =
        json!({"redirect_uris": ["https://a.example/cb"], "client_name": "x".repeat(20_000)});
    let r = send(&app, register_req(too_big.to_string())).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);

    let many: Vec<_> = (0..11).map(|i| format!("https://a.example/{i}")).collect();
    let r = send(
        &app,
        register_req(json!({"redirect_uris": many}).to_string()),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    let r = send(
        &app,
        register_req(json!({"redirect_uris": [uri(2100)]}).to_string()),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"], "invalid_redirect_uri");

    let long_name = json!({"redirect_uris": [uri(5)], "client_name": "n".repeat(201)});
    let r = send(&app, register_req(long_name.to_string())).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"], "invalid_client_metadata");

    let at_limits = json!({"redirect_uris": [uri(2048 - "https://a.example/".len())], "client_name": "n".repeat(200)});
    let r = send(&app, register_req(at_limits.to_string())).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
}

#[tokio::test]
async fn registration_is_rate_limited_per_peer_ip() {
    use axum::extract::ConnectInfo;
    let app = app().await;
    let from = |ip: &str| {
        let mut req = register_req(json!({"redirect_uris": ["https://a.example/cb"]}).to_string());
        let addr: std::net::SocketAddr = format!("{ip}:4000").parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        req
    };

    let mut limited = None;
    for _ in 0..60 {
        let r = send(&app, from("10.0.0.1")).await;
        if r.status == StatusCode::TOO_MANY_REQUESTS {
            limited = Some(r);
            break;
        }
        assert_eq!(r.status, StatusCode::CREATED);
    }
    let limited = limited.expect("a burst of registrations must hit the limit");
    assert!(limited.headers.contains_key(header::RETRY_AFTER));
    assert_eq!(
        send(&app, from("10.0.0.2")).await.status,
        StatusCode::CREATED,
        "other peers are unaffected"
    );
}

#[tokio::test]
async fn setup_is_only_available_while_no_user_exists() {
    let app = app().await;
    let session = setup_and_login(&app).await;
    let again = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    assert_eq!(again.status, StatusCode::SEE_OTHER);
    assert_eq!(again.location(), "/login");
    let csrf = "c".repeat(43);
    let post = send(
        &app,
        form_req(
            "/setup",
            &format!("folio_csrf={csrf}"),
            &format!("csrf={csrf}&username=x&password=another-long-password"),
        ),
    )
    .await;
    assert_eq!(post.location(), "/login");
    assert!(!session.is_empty());
}

#[tokio::test]
async fn setup_requires_the_one_time_token() {
    let app = app().await;
    for uri in ["/setup", "/setup?token=wrong", "/setup?token="] {
        let page = send(&app, Request::get(uri).body(Body::empty()).unwrap()).await;
        assert_eq!(page.status, StatusCode::FORBIDDEN, "{uri}");
        assert!(page.body.contains("Find the setup link in the server log"));
        assert!(!page.body.contains("name=\"password\""));
    }

    // A valid CSRF pair without the right setup token creates nothing.
    let page = send(
        &app,
        Request::get(format!("/setup?token={SETUP_TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let csrf = csrf_from(&page.body);
    for token in ["&token=", "&token=wrong"] {
        let post = send(
            &app,
            form_req(
                "/setup",
                &csrf_cookie(&page),
                &format!("csrf={csrf}{token}&username=evil&password=attacker-password-1"),
            ),
        )
        .await;
        assert_eq!(post.status, StatusCode::FORBIDDEN);
    }
    let login = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
    assert_eq!(login.location(), "/setup", "no user may exist yet");

    // The right token still works, and only once.
    setup_and_login(&app).await;
    let again = send(
        &app,
        Request::get(format!("/setup?token={SETUP_TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(again.location(), "/login");
}

#[tokio::test]
async fn sign_in_page_shares_the_apps_fonts_tokens_and_theme_choice() {
    let app = app().await;
    setup_and_login(&app).await;
    let page = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;

    for asset in [
        "/static/fonts/fonts.css",
        "/static/tokens.css",
        "/static/components.css",
    ] {
        assert!(page.body.contains(asset), "{asset} is linked");
    }
    assert!(
        page.body.contains("folio-theme"),
        "applies the chosen theme"
    );
    assert!(page.body.contains("auth-mark"), "shows the mark");
    assert!(!page.body.contains("f-label"), "labels are sentence case");
}

#[tokio::test]
async fn login_rejects_bad_password_and_bad_csrf_and_open_redirects() {
    let app = app().await;
    setup_and_login(&app).await;
    let page = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
    let csrf = csrf_from(&page.body);
    let cookie = csrf_cookie(&page);

    let bad = send(
        &app,
        form_req(
            "/login",
            &cookie,
            &format!("csrf={csrf}&username=owner&password=wrong"),
        ),
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);

    let forged = send(
        &app,
        form_req(
            "/login",
            &cookie,
            "csrf=forged&username=owner&password=correct-horse-battery",
        ),
    )
    .await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);

    let good = send(
        &app,
        form_req(
            "/login",
            &cookie,
            &format!(
                "csrf={csrf}&username=owner&password=correct-horse-battery&next={}",
                enc("//evil.example")
            ),
        ),
    )
    .await;
    assert_eq!(good.status, StatusCode::SEE_OTHER);
    assert_eq!(good.location(), "/");
    let session_cookie = good
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .next()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(session_cookie.contains("HttpOnly") && session_cookie.contains("SameSite=Lax"));
}

async fn login_attempt(app: &App, username: &str, password: &str, ip: Option<&str>) -> Resp {
    let page = send(app, Request::get("/login").body(Body::empty()).unwrap()).await;
    let csrf = csrf_from(&page.body);
    let mut req = form_req(
        "/login",
        &csrf_cookie(&page),
        &format!(
            "csrf={csrf}&username={}&password={}",
            enc(username),
            enc(password)
        ),
    );
    if let Some(ip) = ip {
        let addr: std::net::SocketAddr = format!("{ip}:5000").parse().unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(addr));
    }
    send(app, req).await
}

#[tokio::test]
async fn repeated_failures_for_one_username_are_locked_out() {
    let app = app().await;
    setup_and_login(&app).await;
    for _ in 0..5 {
        let r = login_attempt(&app, "owner", "wrong-password", None).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    // Even the right password is refused while locked, so guesses cannot be probed.
    let locked = login_attempt(&app, "owner", "correct-horse-battery", None).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = locked.headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(wait > 0);
    // Case-insensitive usernames share one counter.
    let upper = login_attempt(&app, "OWNER", "wrong", None).await;
    assert_eq!(upper.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn repeated_failures_from_one_ip_are_locked_out() {
    let app = app().await;
    setup_and_login(&app).await;
    for i in 0..10 {
        let r = login_attempt(
            &app,
            &format!("user{i}"),
            "wrong-password",
            Some("10.0.0.9"),
        )
        .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "attempt {i}");
    }
    let locked = login_attempt(&app, "owner", "correct-horse-battery", Some("10.0.0.9")).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    let elsewhere = login_attempt(&app, "owner", "correct-horse-battery", Some("10.0.0.10")).await;
    assert_eq!(elsewhere.status, StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn overlong_passwords_are_rejected_without_hashing() {
    let app = app().await;
    setup_and_login(&app).await;
    let huge = "p".repeat(1025);
    let r = login_attempt(&app, "owner", &huge, None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    let fresh = self::app().await;
    let page = send(
        &fresh,
        Request::get(format!("/setup?token={SETUP_TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let csrf = csrf_from(&page.body);
    let r = send(
        &fresh,
        form_req(
            "/setup",
            &csrf_cookie(&page),
            &format!("csrf={csrf}&token={SETUP_TOKEN}&username=owner&password={huge}"),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn auth_pages_cannot_be_framed_or_cached() {
    let app = app().await;
    let setup = send(
        &app,
        Request::get(format!("/setup?token={SETUP_TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let session = setup_and_login(&app).await;
    let client = register(&app).await;
    let consent = send(
        &app,
        get_req(
            &format!("/oauth/authorize?{}", authorize_query(&client)),
            &session,
        ),
    )
    .await;
    let login = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
    for (name, page) in [("setup", setup), ("login", login), ("consent", consent)] {
        assert_eq!(page.status, StatusCode::OK, "{name}");
        assert_eq!(page.headers["x-frame-options"], "DENY", "{name}");
        assert_eq!(
            page.headers[header::CONTENT_SECURITY_POLICY],
            "frame-ancestors 'none'",
            "{name}"
        );
        assert_eq!(page.headers[header::CACHE_CONTROL], "no-store", "{name}");
    }
}

#[tokio::test]
async fn logging_in_ends_the_previous_session() {
    let app = app().await;
    let old = setup_and_login(&app).await;
    let old_session = old.split("; ").last().unwrap().to_string();
    assert_eq!(
        send(&app, get_req("/login", &old_session)).await.status,
        StatusCode::SEE_OTHER,
        "the first session is valid"
    );

    let page = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
    let csrf = csrf_from(&page.body);
    let done = send(
        &app,
        form_req(
            "/login",
            &format!("{}; {old_session}", csrf_cookie(&page)),
            &format!("csrf={csrf}&username=owner&password=correct-horse-battery"),
        ),
    )
    .await;
    assert_eq!(done.status, StatusCode::SEE_OTHER);
    assert_eq!(
        send(&app, get_req("/login", &old_session)).await.status,
        StatusCode::OK,
        "the session presented at login is gone"
    );
    let fresh = done
        .cookies()
        .into_iter()
        .find(|c| c.starts_with("folio_session="))
        .unwrap();
    assert_eq!(
        send(&app, get_req("/login", &fresh)).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn csrf_tokens_are_bound_to_the_server_secret_and_the_session() {
    let app = app().await;
    let session_a = setup_and_login(&app).await;

    // The token is not the cookie value, so planting both cookies is not enough.
    let page = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
    let token = csrf_from(&page.body);
    let planted = csrf_cookie(&page);
    assert_ne!(planted, format!("folio_csrf={token}"));
    let body = |csrf: &str| format!("csrf={csrf}&username=owner&password=correct-horse-battery");
    let forged_cookie = format!("folio_csrf={}", "z".repeat(43));
    let r = send(
        &app,
        form_req("/login", &forged_cookie, &body(&"z".repeat(43))),
    )
    .await;
    assert_eq!(
        r.status,
        StatusCode::FORBIDDEN,
        "self-made cookie+field pair"
    );
    let r = send(&app, form_req("/login", &planted, &body(&token))).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);

    // A consent token minted for one session is useless in another.
    let client = register(&app).await;
    let q = authorize_query(&client);
    let consent_a = send(&app, get_req(&format!("/oauth/authorize?{q}"), &session_a)).await;
    let token_a = csrf_from(&consent_a.body);
    let session_b = r
        .cookies()
        .into_iter()
        .find(|c| c.starts_with("folio_session="))
        .unwrap();
    let post = |session: &str, csrf: &str| {
        form_req(
            "/oauth/authorize",
            session,
            &format!("{q}&csrf={csrf}&decision=allow"),
        )
    };
    let crossed = send(&app, post(&session_b, &token_a)).await;
    assert_eq!(crossed.status, StatusCode::FORBIDDEN);
    let own = send(&app, post(&session_a, &token_a)).await;
    assert_eq!(own.status, StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn https_public_url_uses_host_prefixed_cookies() {
    let app = app_at(Some("https://folio.example.com")).await;
    let page = send(
        &app,
        Request::get(format!("/setup?token={SETUP_TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let set_cookie: Vec<String> = page
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    let csrf = set_cookie
        .iter()
        .find(|c| c.starts_with("__Host-folio_csrf="))
        .expect("csrf cookie is __Host- prefixed");
    assert!(csrf.contains("Secure") && csrf.contains("Path=/") && !csrf.contains("Domain"));

    let done = send(
        &app,
        form_req(
            "/setup",
            &csrf_cookie(&page),
            &format!(
                "csrf={}&token={SETUP_TOKEN}&username=owner&password=correct-horse-battery",
                csrf_from(&page.body)
            ),
        ),
    )
    .await;
    assert_eq!(done.status, StatusCode::SEE_OTHER);
    let session = done
        .cookies()
        .into_iter()
        .find(|c| c.starts_with("__Host-folio_session="))
        .expect("session cookie is __Host- prefixed");
    // The prefixed session is honoured on the next request.
    assert_eq!(
        send(&app, get_req("/login", &session)).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn authorize_redirects_anonymous_users_to_login_and_back() {
    let app = app().await;
    setup_and_login(&app).await;
    let client = register(&app).await;
    let q = authorize_query(&client);
    let res = send(
        &app,
        Request::get(format!("/oauth/authorize?{q}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(res.status, StatusCode::SEE_OTHER);
    assert_eq!(
        res.location(),
        format!("/login?next={}", enc(&format!("/oauth/authorize?{q}")))
    );
}

#[tokio::test]
async fn authorize_rejects_unregistered_redirect_and_missing_pkce() {
    let app = app().await;
    let session = setup_and_login(&app).await;
    let client = register(&app).await;
    let evil = format!("response_type=code&client_id={client}&redirect_uri={}&code_challenge={}&code_challenge_method=S256", enc("https://evil.example/cb"), challenge());
    let res = send(&app, get_req(&format!("/oauth/authorize?{evil}"), &session)).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);

    let no_pkce = format!(
        "response_type=code&client_id={client}&redirect_uri={}",
        enc(REDIRECT)
    );
    let res = send(
        &app,
        get_req(&format!("/oauth/authorize?{no_pkce}"), &session),
    )
    .await;
    assert_eq!(res.status, StatusCode::SEE_OTHER);
    assert!(res.location().contains("error=invalid_request"));
    assert!(res.location().contains(&format!("iss={}", enc(BASE))));
}

#[tokio::test]
async fn full_flow_yields_a_token_the_validator_accepts() {
    let app = app().await;
    let session = setup_and_login(&app).await;
    let client = register(&app).await;

    let no_token = send(&app, Request::get("/mcp").body(Body::empty()).unwrap()).await;
    assert_eq!(no_token.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        no_token.headers[header::WWW_AUTHENTICATE],
        format!("Bearer resource_metadata=\"{BASE}/.well-known/oauth-protected-resource/mcp\"")
            .as_str()
    );

    let code = obtain_code(&app, &session, &client).await;
    let tokens = exchange(&app, &code_body(&client, &code, VERIFIER)).await;
    assert_eq!(tokens.status, StatusCode::OK, "{}", tokens.body);
    let tokens = tokens.json();
    assert_eq!(tokens["token_type"], "Bearer");
    assert_eq!(tokens["expires_in"], 3600);
    assert_eq!(tokens["scope"], "documents:read documents:write");

    let access = tokens["access_token"].as_str().unwrap();
    let ok = send(
        &app,
        Request::get("/mcp")
            .header(header::AUTHORIZATION, format!("Bearer {access}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK);

    let bogus = send(
        &app,
        Request::get("/mcp")
            .header(header::AUTHORIZATION, "Bearer nope")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(bogus.status, StatusCode::UNAUTHORIZED);
    assert!(bogus.headers[header::WWW_AUTHENTICATE]
        .to_str()
        .unwrap()
        .contains("invalid_token"));

    // Revocation invalidates the token.
    let revoked = send(
        &app,
        form_req(
            "/oauth/revoke",
            "",
            &format!("token={access}&client_id={client}"),
        ),
    )
    .await;
    assert_eq!(revoked.status, StatusCode::OK);
    let after = send(
        &app,
        Request::get("/mcp")
            .header(header::AUTHORIZATION, format!("Bearer {access}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(after.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_verifier_fails_and_burns_the_code() {
    let app = app().await;
    let session = setup_and_login(&app).await;
    let client = register(&app).await;
    let code = obtain_code(&app, &session, &client).await;
    let wrong = "a".repeat(43);
    let bad = exchange(&app, &code_body(&client, &code, &wrong)).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad.json()["error"], "invalid_grant");
    let retry = exchange(&app, &code_body(&client, &code, VERIFIER)).await;
    assert_eq!(retry.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn code_cannot_be_reused() {
    let app = app().await;
    let session = setup_and_login(&app).await;
    let client = register(&app).await;
    let code = obtain_code(&app, &session, &client).await;
    assert_eq!(
        exchange(&app, &code_body(&client, &code, VERIFIER))
            .await
            .status,
        StatusCode::OK
    );
    let again = exchange(&app, &code_body(&client, &code, VERIFIER)).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    assert_eq!(again.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn token_exchange_requires_exact_redirect_uri() {
    let app = app().await;
    let session = setup_and_login(&app).await;
    let client = register(&app).await;
    let code = obtain_code(&app, &session, &client).await;
    let body = code_body(&client, &code, VERIFIER)
        .replace(&enc(REDIRECT), &enc("http://127.0.0.1:1/callback"));
    let res = exchange(&app, &body).await;
    assert_eq!(res.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn token_for_other_resource_is_rejected() {
    let app = app().await;
    let session = setup_and_login(&app).await;
    let client = register(&app).await;
    let code = obtain_code(&app, &session, &client).await;
    let body = code_body(&client, &code, VERIFIER).replace(
        &enc(&format!("{BASE}/mcp")),
        &enc("https://other.example/mcp"),
    );
    assert_eq!(
        exchange(&app, &body).await.json()["error"],
        "invalid_target"
    );
}

#[tokio::test]
async fn refresh_rotation_invalidates_the_old_refresh_token() {
    let app = app().await;
    let session = setup_and_login(&app).await;
    let client = register(&app).await;
    let code = obtain_code(&app, &session, &client).await;
    let first = exchange(&app, &code_body(&client, &code, VERIFIER))
        .await
        .json();
    let old = first["refresh_token"].as_str().unwrap();

    let refresh =
        |t: &str| format!("grant_type=refresh_token&client_id={client}&refresh_token={t}");
    let second = exchange(&app, &refresh(old)).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    let second = second.json();
    assert_ne!(second["refresh_token"], first["refresh_token"]);

    let new_access = second["access_token"].as_str().unwrap();
    assert_eq!(mcp_status(&app, new_access).await, StatusCode::OK);

    let replay = exchange(&app, &refresh(old)).await;
    assert_eq!(replay.json()["error"], "invalid_grant");
}

async fn mcp_status(app: &App, access: &str) -> StatusCode {
    send(
        app,
        Request::get("/mcp")
            .header(header::AUTHORIZATION, format!("Bearer {access}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .status
}

#[tokio::test]
async fn refresh_token_reuse_revokes_the_whole_grant() {
    let app = app().await;
    let session = setup_and_login(&app).await;
    let client = register(&app).await;
    let code = obtain_code(&app, &session, &client).await;
    let first = exchange(&app, &code_body(&client, &code, VERIFIER))
        .await
        .json();
    let refresh =
        |t: &str| format!("grant_type=refresh_token&client_id={client}&refresh_token={t}");
    let second = exchange(&app, &refresh(first["refresh_token"].as_str().unwrap()))
        .await
        .json();
    let (access1, access2) = (
        first["access_token"].as_str().unwrap(),
        second["access_token"].as_str().unwrap(),
    );
    assert_eq!(mcp_status(&app, access2).await, StatusCode::OK);

    // A stolen, already-rotated refresh token shows up again.
    let replay = exchange(&app, &refresh(first["refresh_token"].as_str().unwrap())).await;
    assert_eq!(replay.json()["error"], "invalid_grant");

    assert_eq!(mcp_status(&app, access1).await, StatusCode::UNAUTHORIZED);
    assert_eq!(mcp_status(&app, access2).await, StatusCode::UNAUTHORIZED);
    let live = exchange(&app, &refresh(second["refresh_token"].as_str().unwrap())).await;
    assert_eq!(live.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn revoke_checks_the_client_and_kills_the_whole_grant() {
    let app = app().await;
    let session = setup_and_login(&app).await;
    let client = register(&app).await;
    let other = register(&app).await;
    let code = obtain_code(&app, &session, &client).await;
    let tokens = exchange(&app, &code_body(&client, &code, VERIFIER))
        .await
        .json();
    let (access, refresh) = (
        tokens["access_token"].as_str().unwrap(),
        tokens["refresh_token"].as_str().unwrap(),
    );
    let revoke = |client_id: &str| {
        form_req(
            "/oauth/revoke",
            "",
            &format!("token={refresh}&client_id={client_id}"),
        )
    };

    // Another client cannot revoke it (and learns nothing).
    assert_eq!(send(&app, revoke(&other)).await.status, StatusCode::OK);
    assert_eq!(mcp_status(&app, access).await, StatusCode::OK);

    assert_eq!(send(&app, revoke(&client)).await.status, StatusCode::OK);
    assert_eq!(mcp_status(&app, access).await, StatusCode::UNAUTHORIZED);
    let refreshed = exchange(
        &app,
        &format!("grant_type=refresh_token&client_id={client}&refresh_token={refresh}"),
    )
    .await;
    assert_eq!(refreshed.json()["error"], "invalid_grant");
}

#[tokio::test]
async fn denying_consent_returns_access_denied() {
    let app = app().await;
    let session = setup_and_login(&app).await;
    let client = register(&app).await;
    let q = authorize_query(&client);
    let page = send(&app, get_req(&format!("/oauth/authorize?{q}"), &session)).await;
    let csrf = csrf_from(&page.body);
    let done = send(
        &app,
        form_req(
            "/oauth/authorize",
            &session,
            &format!("{q}&csrf={csrf}&decision=deny"),
        ),
    )
    .await;
    assert!(done.location().contains("error=access_denied"));
    assert!(done.location().contains("state=xyz"));
    assert!(done.location().contains(&format!("iss={}", enc(BASE))));
}

mod enforcement {
    use super::*;
    use folio_api::enforce_auth;

    async fn guarded(required: bool) -> App {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite:{}?mode=rwc", dir.path().join("t.db").display());
        let pool = create_pool(&url).await.unwrap();
        run_migrations(&pool).await.unwrap();
        let auth = AuthState::new(Arc::new(SqliteAuthStore::new(pool)), Some(BASE.into()))
            .with_setup_token(SETUP_TOKEN)
            .with_required(required);
        let mut pages = Router::new();
        for path in [
            "/",
            "/healthz",
            "/readyz",
            "/static/app.css",
            "/api/documents",
            "/api/openapi.json",
            "/api/docs",
            "/mcp",
            "/manifest.webmanifest",
            "/sw.js",
        ] {
            pages = pages.route(path, get(|| async { "ok" }));
        }
        pages = pages
            .route("/api/tags", axum::routing::post(|| async { "created" }))
            .route("/share", axum::routing::post(|| async { "stored" }));
        App {
            router: auth_router(auth.clone())
                .merge(pages)
                .layer(middleware::from_fn_with_state(auth, enforce_auth)),
            _dir: dir,
        }
    }

    async fn status(app: &App, uri: &str) -> StatusCode {
        send(app, Request::get(uri).body(Body::empty()).unwrap())
            .await
            .status
    }

    #[tokio::test]
    async fn exempt_paths_are_open() {
        let app = guarded(true).await;
        for path in [
            "/healthz",
            "/readyz",
            "/static/app.css",
            "/api/openapi.json",
            "/api/docs",
            "/mcp",
            "/auth/theme.css",
            "/.well-known/oauth-authorization-server",
            "/.well-known/oauth-protected-resource/mcp",
        ] {
            assert_eq!(status(&app, path).await, StatusCode::OK, "{path}");
        }
    }

    #[tokio::test]
    async fn oauth_endpoints_and_login_are_open() {
        let app = guarded(true).await;
        setup_and_login(&app).await;
        assert_eq!(status(&app, "/login").await, StatusCode::OK);
        let token = send(
            &app,
            form_req("/oauth/token", "", "grant_type=bogus&client_id=x"),
        )
        .await;
        assert_eq!(token.status, StatusCode::UNAUTHORIZED);
        assert_eq!(token.json()["error"], "invalid_client");
    }

    #[tokio::test]
    async fn setup_is_open_only_while_no_user_exists() {
        let app = guarded(true).await;
        assert_eq!(
            status(&app, &format!("/setup?token={SETUP_TOKEN}")).await,
            StatusCode::OK
        );
        setup_and_login(&app).await;
        let after = send(&app, Request::get("/setup").body(Body::empty()).unwrap()).await;
        assert_eq!(after.status, StatusCode::SEE_OTHER);
        assert!(after.location().starts_with("/login"));
    }

    #[tokio::test]
    async fn web_pages_redirect_to_login_with_next() {
        let app = guarded(true).await;
        let res = send(&app, Request::get("/?q=a").body(Body::empty()).unwrap()).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER);
        assert_eq!(res.location(), format!("/login?next={}", enc("/?q=a")));
    }

    #[tokio::test]
    async fn api_gets_401_json_with_challenge() {
        let app = guarded(true).await;
        let res = send(
            &app,
            Request::get("/api/documents").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
        assert_eq!(res.json()["error"], "unauthorized");
        assert!(res.headers[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap()
            .starts_with("Bearer resource_metadata="));
    }

    #[tokio::test]
    async fn session_and_api_token_grant_access() {
        let app = guarded(true).await;
        let session = setup_and_login(&app).await;
        assert_eq!(
            send(&app, get_req("/", &session)).await.status,
            StatusCode::OK
        );
        assert_eq!(
            send(&app, get_req("/api/documents", &session)).await.status,
            StatusCode::OK
        );

        let client = register(&app).await;
        let code = obtain_code(&app, &session, &client).await;
        let body = code_body(&client, &code, VERIFIER);
        let tokens = exchange(&app, &body).await.json();
        let access = tokens["access_token"].as_str().unwrap();
        // The token is bound to /mcp, so the REST API rejects it.
        let res = send(
            &app,
            Request::get("/api/documents")
                .header(header::AUTHORIZATION, format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn api_scoped_token_grants_api_access() {
        let app = guarded(true).await;
        let session = setup_and_login(&app).await;
        let client = register(&app).await;
        let api = enc(&format!("{BASE}/api"));
        let q = authorize_query(&client).replace(&enc(&format!("{BASE}/mcp")), &api);
        let page = send(&app, get_req(&format!("/oauth/authorize?{q}"), &session)).await;
        let csrf = csrf_from(&page.body);
        let done = send(
            &app,
            form_req(
                "/oauth/authorize",
                &session,
                &format!("{q}&csrf={csrf}&decision=allow"),
            ),
        )
        .await;
        let loc = url::Url::parse(&done.location()).unwrap();
        let code = loc
            .query_pairs()
            .find(|(k, _)| k == "code")
            .unwrap()
            .1
            .to_string();
        let body = code_body(&client, &code, VERIFIER).replace(&enc(&format!("{BASE}/mcp")), &api);
        let tokens = exchange(&app, &body).await.json();
        let access = tokens["access_token"].as_str().unwrap();
        let res = send(
            &app,
            Request::get("/api/documents")
                .header(header::AUTHORIZATION, format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(res.status, StatusCode::OK);
    }

    #[tokio::test]
    async fn unsuffixed_resource_metadata_is_not_routed() {
        let app = guarded(false).await;
        assert_eq!(
            status(&app, "/.well-known/oauth-protected-resource").await,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn auth_can_be_disabled() {
        let app = guarded(false).await;
        assert_eq!(status(&app, "/").await, StatusCode::OK);
        assert_eq!(status(&app, "/api/documents").await, StatusCode::OK);
    }

    #[tokio::test]
    async fn pwa_assets_are_public_and_anonymous_shares_are_not_accepted() {
        let app = guarded(true).await;
        setup_and_login(&app).await;
        for path in ["/manifest.webmanifest", "/sw.js"] {
            assert_eq!(status(&app, path).await, StatusCode::OK, "{path}");
        }

        let share = |cookie: &str| {
            Request::post("/share")
                .header(header::CONTENT_TYPE, "multipart/form-data; boundary=x")
                .header(header::COOKIE, cookie)
                .body(Body::from("--x--\r\n"))
                .unwrap()
        };
        let anonymous = send(&app, share("")).await;
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
        assert!(anonymous.body.contains("share again"));
        assert!(anonymous.body.contains("/login"));
    }

    #[tokio::test]
    async fn api_challenges_point_at_the_api_resource_metadata() {
        let app = guarded(true).await;
        let prm = send(
            &app,
            Request::get("/.well-known/oauth-protected-resource/api")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(prm.status, StatusCode::OK);
        assert_eq!(prm.json()["resource"], format!("{BASE}/api"));
        assert_eq!(prm.json()["authorization_servers"], json!([BASE]));

        let denied = send(
            &app,
            Request::get("/api/documents").body(Body::empty()).unwrap(),
        )
        .await;
        assert!(denied.headers[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap()
            .contains(&format!(
                "resource_metadata=\"{BASE}/.well-known/oauth-protected-resource/api\""
            )));
    }

    #[tokio::test]
    async fn cookie_writes_must_be_same_origin() {
        let app = guarded(true).await;
        let session = setup_and_login(&app).await;
        let write = |extra: &[(&str, &str)]| {
            let mut req = Request::post("/api/tags").header(header::COOKIE, &session);
            for (k, v) in extra {
                req = req.header(*k, *v);
            }
            req.body(Body::empty()).unwrap()
        };
        let ok = [
            vec![],
            vec![("sec-fetch-site", "same-origin")],
            vec![("sec-fetch-site", "none")],
            vec![("origin", BASE)],
        ];
        for headers in ok {
            let r = send(&app, write(&headers)).await;
            assert_eq!(r.status, StatusCode::OK, "{headers:?}");
        }
        let forbidden = [
            vec![("sec-fetch-site", "cross-site")],
            vec![("sec-fetch-site", "same-site")],
            vec![("origin", "https://evil.example")],
            vec![("origin", "null")],
            // Fetch metadata wins over a spoofable-looking Origin.
            vec![("sec-fetch-site", "cross-site"), ("origin", BASE)],
        ];
        for headers in forbidden {
            let r = send(&app, write(&headers)).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{headers:?}");
        }
        // Reads are not state-changing.
        let read = Request::get("/api/documents")
            .header(header::COOKIE, &session)
            .header("sec-fetch-site", "cross-site")
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&app, read).await.status, StatusCode::OK);
    }

    /// Runs consent for an `/api` audience with the given scopes and returns the access token.
    async fn api_token(app: &App, session: &str, scope: &str) -> String {
        let client = register(app).await;
        let api = enc(&format!("{BASE}/api"));
        let q = format!(
            "{}&scope={}",
            authorize_query(&client).replace(&enc(&format!("{BASE}/mcp")), &api),
            enc(scope)
        );
        let page = send(app, get_req(&format!("/oauth/authorize?{q}"), session)).await;
        let csrf = csrf_from(&page.body);
        let done = send(
            app,
            form_req(
                "/oauth/authorize",
                session,
                &format!("{q}&csrf={csrf}&decision=allow"),
            ),
        )
        .await;
        let loc = url::Url::parse(&done.location()).unwrap();
        let code = loc
            .query_pairs()
            .find(|(k, _)| k == "code")
            .unwrap()
            .1
            .to_string();
        let body = code_body(&client, &code, VERIFIER).replace(&enc(&format!("{BASE}/mcp")), &api);
        let tokens = exchange(app, &body).await.json();
        tokens["access_token"].as_str().unwrap().to_string()
    }

    fn bearer(method: &str, uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn read_only_token_can_reach_client_telemetry_without_document_write_scope() {
        let app = guarded(true).await;
        let session = setup_and_login(&app).await;
        let read_only = api_token(&app, &session, "documents:read").await;
        let response = send(&app, bearer("POST", "/api/telemetry/v1/traces", &read_only)).await;
        assert_eq!(
            response.status,
            StatusCode::NOT_FOUND,
            "disabled telemetry is reached after authentication"
        );
    }

    #[tokio::test]
    async fn read_only_token_cannot_write_through_the_api() {
        let app = guarded(true).await;
        let session = setup_and_login(&app).await;
        let read_only = api_token(&app, &session, "documents:read").await;

        let read = send(&app, bearer("GET", "/api/documents", &read_only)).await;
        assert_eq!(read.status, StatusCode::OK);
        let write = send(&app, bearer("POST", "/api/tags", &read_only)).await;
        assert_eq!(write.status, StatusCode::FORBIDDEN);
        assert_eq!(write.json()["error"], "insufficient_scope");
        assert!(write.headers[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap()
            .contains("insufficient_scope"));

        let writer = api_token(&app, &session, "documents:write").await;
        assert_eq!(
            send(&app, bearer("POST", "/api/tags", &writer))
                .await
                .status,
            StatusCode::OK
        );
        let read_with_write_only = send(&app, bearer("GET", "/api/documents", &writer)).await;
        assert_eq!(read_with_write_only.status, StatusCode::FORBIDDEN);
    }
}

mod forward {
    use super::*;
    use axum::extract::ConnectInfo;
    use axum::http::HeaderMap;
    use folio_api::{enforce_auth, forward_auth};
    use folio_auth::{AuthStore, ForwardAuth, ForwardConfig, GroupSource};
    use folio_infra::Database;
    use std::net::SocketAddr;

    const TRUSTED: &str = "10.1.2.3:5555";
    const UNTRUSTED: &str = "203.0.113.9:5555";

    struct Fwd {
        app: App,
        store: Arc<dyn AuthStore>,
        pool: sqlx::SqlitePool,
    }

    async fn fwd_app(configure: impl FnOnce(&mut ForwardConfig)) -> Fwd {
        fwd_app_at(BASE, configure).await
    }

    async fn fwd_app_at(base: &str, configure: impl FnOnce(&mut ForwardConfig)) -> Fwd {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::connect(&format!("sqlite:{}/t.db?mode=rwc", dir.path().display()))
            .await
            .unwrap();
        db.migrate().await.unwrap();
        let Database::Sqlite(pool) = db.clone() else {
            unreachable!()
        };
        let store = db.repositories().await.auth;
        let mut config = ForwardConfig {
            trusted_proxies: vec!["10.0.0.0/8".to_string()],
            ..ForwardConfig::default()
        };
        configure(&mut config);
        let forward = ForwardAuth::new(config).unwrap();
        let auth = AuthState::new(store.clone(), Some(base.into()))
            .with_setup_token(SETUP_TOKEN)
            .with_forward(forward);
        let router = auth_router(auth.clone())
            .merge(Router::new().route("/", get(|| async { "home" })).route(
                "/static/echo",
                get(|headers: HeaderMap| async move {
                    let mut names: Vec<_> =
                        headers.keys().map(|k| k.as_str().to_string()).collect();
                    names.sort();
                    names.join(",")
                }),
            ))
            .layer(middleware::from_fn_with_state(auth.clone(), enforce_auth))
            .layer(middleware::from_fn_with_state(auth, forward_auth));
        Fwd {
            app: App { router, _dir: dir },
            store,
            pool,
        }
    }

    async fn send_from(app: &App, peer: &str, mut req: Request<Body>) -> Resp {
        let addr: SocketAddr = peer.parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        send(app, req).await
    }

    fn with_identity(uri: &str, user: &str, extra: &[(&str, &str)]) -> Request<Body> {
        let mut b = Request::get(uri)
            .header("Accept", "text/html")
            .header("Remote-User", user);
        for (k, v) in extra {
            b = b.header(*k, *v);
        }
        b.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn headers_from_an_untrusted_peer_are_ignored() {
        let f = fwd_app(|_| {}).await;
        let res = send_from(&f.app, UNTRUSTED, with_identity("/", "alice", &[])).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER);
        assert!(res.location().starts_with("/login"));
        assert!(res.cookies().is_empty());
        assert_eq!(f.store.count_users().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn requests_without_a_peer_address_are_not_trusted() {
        let f = fwd_app(|_| {}).await;
        let res = send(&f.app, with_identity("/", "alice", &[])).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER);
        assert_eq!(f.store.count_users().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn trusted_peer_without_header_is_not_logged_in() {
        let f = fwd_app(|_| {}).await;
        let res = send_from(
            &f.app,
            TRUSTED,
            Request::get("/").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(res.status, StatusCode::SEE_OTHER);
    }

    #[tokio::test]
    async fn trusted_peer_gets_a_session_and_the_first_user_is_admin() {
        let f = fwd_app(|_| {}).await;
        let req = with_identity(
            "/",
            "Alice",
            &[
                ("Remote-Email", "alice@example.com"),
                ("Remote-Name", "Alice A."),
            ],
        );
        let res = send_from(&f.app, TRUSTED, req).await;
        assert_eq!(res.status, StatusCode::OK);
        let session = res
            .cookies()
            .into_iter()
            .find(|c| c.contains("folio_session="))
            .expect("session cookie");

        let alice = f
            .store
            .find_user_by_external("forward", "alice")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(alice.username, "Alice");
        assert_eq!(alice.email.as_deref(), Some("alice@example.com"));
        assert_eq!(alice.display_name.as_deref(), Some("Alice A."));
        assert!(alice.is_admin, "the first user becomes the owner");

        // The cookie alone keeps working, from any peer and without the header.
        let again = send_from(&f.app, UNTRUSTED, get_req("/", &session)).await;
        assert_eq!(again.status, StatusCode::OK);

        // A repeat request through the proxy reuses the session.
        let repeat = send_from(
            &f.app,
            TRUSTED,
            with_identity("/", "Alice", &[("Cookie", &session)]),
        )
        .await;
        assert_eq!(repeat.status, StatusCode::OK);
        assert!(repeat.cookies().is_empty());
    }

    #[tokio::test]
    async fn admin_group_maps_to_admin_and_follows_the_idp() {
        let f = fwd_app(|c| {
            c.groups_header = "Remote-Groups".into();
            c.admin_group = Some("admins".into());
        })
        .await;
        send_from(
            &f.app,
            TRUSTED,
            with_identity("/", "root", &[("Remote-Groups", "admins,family")]),
        )
        .await;
        let bob = send_from(
            &f.app,
            TRUSTED,
            with_identity("/", "bob", &[("Remote-Groups", "family")]),
        )
        .await;
        assert_eq!(bob.status, StatusCode::OK);
        let user = |name: &'static str| {
            let store = f.store.clone();
            async move {
                store
                    .find_user_by_external("forward", name)
                    .await
                    .unwrap()
                    .unwrap()
            }
        };
        assert!(user("root").await.is_admin);
        assert!(!user("bob").await.is_admin);

        // Promoted when added to the group, demoted when removed.
        send_from(
            &f.app,
            TRUSTED,
            with_identity("/", "bob", &[("Remote-Groups", "family,admins")]),
        )
        .await;
        assert!(user("bob").await.is_admin);
        send_from(
            &f.app,
            TRUSTED,
            with_identity("/", "bob", &[("Remote-Groups", "family")]),
        )
        .await;
        assert!(!user("bob").await.is_admin);
    }

    #[tokio::test]
    async fn a_forged_groups_header_cannot_grant_admin_by_default() {
        // Pangolin's Badger does not strip Remote-Groups, so a client can send it.
        let f = fwd_app(|c| c.admin_group = Some("folio-admins".into())).await;
        send_from(&f.app, TRUSTED, with_identity("/", "owner", &[])).await;
        let forged = with_identity("/", "mallory", &[("Remote-Groups", "folio-admins")]);
        assert_eq!(
            send_from(&f.app, TRUSTED, forged).await.status,
            StatusCode::OK
        );
        let mallory = f
            .store
            .find_user_by_external("forward", "mallory")
            .await
            .unwrap()
            .unwrap();
        assert!(!mallory.is_admin);
        assert!(f.store.user_groups(&mallory.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn admin_role_header_maps_to_admin_and_follows_the_proxy() {
        let f = fwd_app(|c| {
            c.role_header = "Remote-Role".into();
            c.admin_role = Some("admin".into());
        })
        .await;
        let as_role = |role: &'static str| with_identity("/", "pat", &[("Remote-Role", role)]);
        let pat = || async {
            f.store
                .find_user_by_external("forward", "pat")
                .await
                .unwrap()
                .unwrap()
        };
        send_from(&f.app, TRUSTED, with_identity("/", "owner", &[])).await;
        send_from(&f.app, TRUSTED, as_role("member")).await;
        assert!(!pat().await.is_admin);
        send_from(&f.app, TRUSTED, as_role("admin")).await;
        assert!(pat().await.is_admin);
        send_from(&f.app, TRUSTED, as_role("member")).await;
        assert!(!pat().await.is_admin);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn simultaneous_first_requests_for_one_user_all_succeed() {
        let f = fwd_app(|_| {}).await;
        let statuses: Vec<StatusCode> = futures::future::join_all(
            (0..16).map(|_| send_from(&f.app, TRUSTED, with_identity("/", "newcomer", &[]))),
        )
        .await
        .into_iter()
        .map(|r| r.status)
        .collect();
        assert!(
            statuses.iter().all(|s| *s == StatusCode::OK),
            "{statuses:?}"
        );
        assert_eq!(f.store.count_users().await.unwrap(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn only_one_of_several_simultaneous_first_users_becomes_admin() {
        let f = fwd_app(|_| {}).await;
        futures::future::join_all((0..16).map(|i| {
            send_from(
                &f.app,
                TRUSTED,
                with_identity("/", &format!("user{i}"), &[]),
            )
        }))
        .await;
        let admins: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE is_admin = 1")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert_eq!(f.store.count_users().await.unwrap(), 16);
        assert_eq!(admins, 1);
    }

    #[tokio::test]
    async fn the_owner_is_never_demoted_by_a_mapping() {
        let f = fwd_app(|c| {
            c.role_header = "Remote-Role".into();
            c.admin_role = Some("admin".into());
        })
        .await;
        let user = |name: &'static str| {
            let store = f.store.clone();
            async move {
                store
                    .find_user_by_external("forward", name)
                    .await
                    .unwrap()
                    .unwrap()
            }
        };
        for _ in 0..2 {
            let req = with_identity("/", "first", &[("Remote-Role", "member")]);
            send_from(&f.app, TRUSTED, req).await;
            let first = user("first").await;
            assert!(first.is_owner && first.is_admin, "{first:?}");
        }
        let req = with_identity("/", "second", &[("Remote-Role", "member")]);
        send_from(&f.app, TRUSTED, req).await;
        let second = user("second").await;
        assert!(!second.is_owner && !second.is_admin);
    }

    fn all_identity_headers(c: &mut ForwardConfig) {
        c.groups_header = "Remote-Groups".into();
        c.role_header = "Remote-Role".into();
        c.admin_group = Some("admins".into());
    }

    #[tokio::test]
    async fn every_identity_header_is_stripped_from_untrusted_peers() {
        let f = fwd_app(all_identity_headers).await;
        // No user header at all: the other headers must still not get through.
        let req = Request::get("/static/echo")
            .header("Remote-Email", "x@example.com")
            .header("Remote-Name", "X")
            .header("Remote-Groups", "admins")
            .header("Remote-Role", "admin")
            .header("X-Keep", "1")
            .body(Body::empty())
            .unwrap();
        let res = send_from(&f.app, UNTRUSTED, req).await;
        assert_eq!(res.body, "x-keep");
    }

    #[tokio::test]
    async fn trusted_requests_without_a_user_header_lose_the_other_identity_headers() {
        let f = fwd_app(all_identity_headers).await;
        let req = Request::get("/static/echo")
            .header("Remote-Email", "x@example.com")
            .header("Remote-Groups", "admins")
            .body(Body::empty())
            .unwrap();
        let res = send_from(&f.app, TRUSTED, req).await;
        assert_eq!(res.body, "");
    }

    #[tokio::test]
    async fn repeated_identity_headers_are_rejected() {
        let f = fwd_app(all_identity_headers).await;
        let doubled = |name: &str| {
            Request::get("/")
                .header("Remote-User", "alice")
                .header(name, "one")
                .header(name, "two")
                .body(Body::empty())
                .unwrap()
        };
        for name in [
            "Remote-User",
            "Remote-Email",
            "Remote-Name",
            "Remote-Groups",
            "Remote-Role",
        ] {
            let res = send_from(&f.app, TRUSTED, doubled(name)).await;
            assert_eq!(res.status, StatusCode::BAD_REQUEST, "{name}");
        }
        assert_eq!(f.store.count_users().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn machine_paths_never_honour_forward_headers() {
        let f = fwd_app(|_| {}).await;
        let cases = [
            ("GET", "/.well-known/oauth-authorization-server", None),
            ("POST", "/oauth/token", None),
            ("POST", "/oauth/register", None),
            ("POST", "/oauth/revoke", None),
            ("GET", "/mcp", None),
            ("GET", "/mcp/sse", None),
            ("GET", "/api/documents", Some("Bearer not-a-real-token")),
        ];
        for (method, path, bearer) in cases {
            let mut b = Request::builder()
                .method(method)
                .uri(path)
                .header("Remote-User", "mallory");
            if let Some(bearer) = bearer {
                b = b.header(header::AUTHORIZATION, bearer);
            }
            let res = send_from(&f.app, TRUSTED, b.body(Body::empty()).unwrap()).await;
            assert!(res.cookies().is_empty(), "{path} set a session");
            if path.starts_with("/api") {
                assert_eq!(res.status, StatusCode::UNAUTHORIZED, "{path}");
            }
        }
        assert_eq!(f.store.count_users().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_configured_shared_secret_must_accompany_the_identity() {
        let f = fwd_app(|c| c.shared_secret = Some("s3cret-value".into())).await;
        let echo = |secret: Option<&str>| {
            let mut b = Request::get("/static/echo").header("Remote-User", "alice");
            if let Some(secret) = secret {
                b = b.header("X-Folio-Proxy-Secret", secret);
            }
            b.body(Body::empty()).unwrap()
        };
        for secret in [None, Some("wrong"), Some("s3cret-value-and-more")] {
            let res = send_from(&f.app, TRUSTED, echo(secret)).await;
            assert!(res.cookies().is_empty(), "{secret:?}");
            assert_eq!(res.body, "", "{secret:?}: headers must not leak through");
        }
        assert_eq!(f.store.count_users().await.unwrap(), 0);

        let ok = send_from(&f.app, TRUSTED, echo(Some("s3cret-value"))).await;
        assert_eq!(ok.status, StatusCode::OK);
        assert!(
            !ok.body.contains("secret"),
            "the secret header is not passed on: {}",
            ok.body
        );
        assert_eq!(f.store.count_users().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_stable_id_header_links_the_account_and_the_name_is_display_only() {
        let f = fwd_app(|c| c.user_id_header = "Remote-User-Id".into()).await;
        let as_user = |id: Option<&str>, name: &str| {
            let mut b = Request::get("/")
                .header("Accept", "text/html")
                .header("Remote-User", name);
            if let Some(id) = id {
                b = b.header("Remote-User-Id", id);
            }
            b.body(Body::empty()).unwrap()
        };
        let first = send_from(&f.app, TRUSTED, as_user(Some("u-1"), "alice")).await;
        assert_eq!(first.status, StatusCode::OK);
        let alice = f
            .store
            .find_user_by_external("forward", "u-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(alice.username, "alice");

        // The same id under another display name is still the same account.
        let renamed = send_from(&f.app, TRUSTED, as_user(Some("u-1"), "alice-renamed")).await;
        assert_eq!(renamed.status, StatusCode::OK);
        assert_eq!(f.store.count_users().await.unwrap(), 1);

        // Another id cannot take over the name.
        let clash = send_from(&f.app, TRUSTED, as_user(Some("u-2"), "alice")).await;
        assert_eq!(clash.status, StatusCode::FORBIDDEN);

        // Without the id header there is no identity at all.
        let anonymous = send_from(&f.app, TRUSTED, as_user(None, "alice")).await;
        assert_eq!(anonymous.status, StatusCode::SEE_OTHER);
        assert_eq!(f.store.count_users().await.unwrap(), 1);
    }

    async fn session_count(f: &Fwd) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(&f.pool)
            .await
            .unwrap()
    }

    fn session_cookie(res: &Resp) -> String {
        res.cookies()
            .into_iter()
            .find(|c| c.contains("folio_session="))
            .expect("session cookie")
    }

    #[tokio::test]
    async fn forward_sessions_are_short_lived_and_reused() {
        let f = fwd_app(|_| {}).await;
        let first = send_from(&f.app, TRUSTED, with_identity("/", "alice", &[])).await;
        let set_cookie = first
            .headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .find(|c| c.starts_with("folio_session="))
            .unwrap();
        assert!(set_cookie.contains("Max-Age=43200"), "{set_cookie}");
        let expires: i64 = sqlx::query_scalar("SELECT expires_at FROM sessions")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!((expires - now - 43200).abs() < 60, "{}", expires - now);

        let cookie = session_cookie(&first);
        for _ in 0..3 {
            send_from(
                &f.app,
                TRUSTED,
                with_identity("/", "alice", &[("Cookie", &cookie)]),
            )
            .await;
        }
        assert_eq!(session_count(&f).await, 1);
    }

    #[tokio::test]
    async fn requests_that_are_not_page_loads_do_not_create_sessions() {
        let f = fwd_app(|_| {}).await;
        for _ in 0..16 {
            let req = Request::get("/")
                .header("Accept", "application/json")
                .header("Remote-User", "alice")
                .body(Body::empty())
                .unwrap();
            let res = send_from(&f.app, TRUSTED, req).await;
            assert!(res.cookies().is_empty());
        }
        assert_eq!(session_count(&f).await, 0);
    }

    #[tokio::test]
    async fn a_forward_session_ends_when_the_proxy_stops_vouching_for_the_user() {
        let f = fwd_app(|_| {}).await;
        let login = send_from(&f.app, TRUSTED, with_identity("/", "alice", &[])).await;
        let cookie = session_cookie(&login);
        // Any other peer keeps working with the cookie alone.
        let elsewhere = send_from(&f.app, UNTRUSTED, get_req("/", &cookie)).await;
        assert_eq!(elsewhere.status, StatusCode::OK);

        // The proxy now forwards the browser without an identity: signed out upstream.
        let anonymous = Request::get("/")
            .header("Cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let res = send_from(&f.app, TRUSTED, anonymous).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER);
        assert_eq!(session_count(&f).await, 0);
        let again = send_from(&f.app, UNTRUSTED, get_req("/", &cookie)).await;
        assert_eq!(again.status, StatusCode::SEE_OTHER);
    }

    #[tokio::test]
    async fn forward_sessions_pass_the_csrf_and_same_origin_checks() {
        let f = fwd_app(|_| {}).await;
        let client = register(&f.app).await;
        let q = authorize_query(&client);
        let uri = format!("/oauth/authorize?{q}");
        let page = send_from(&f.app, TRUSTED, with_identity(&uri, "erin", &[])).await;
        assert_eq!(page.status, StatusCode::OK, "{}", page.body);
        let session = session_cookie(&page);
        let csrf = csrf_from(&page.body);

        let post = |uri: &str, body: String, site: &str| {
            let mut req = form_req(uri, &session, &body);
            req.headers_mut()
                .insert("Remote-User", "erin".parse().unwrap());
            req.headers_mut()
                .insert("sec-fetch-site", site.parse().unwrap());
            req
        };
        // The session-bound CSRF token from the consent page is accepted.
        let body = format!("{q}&csrf={csrf}&decision=allow");
        let own = send_from(
            &f.app,
            TRUSTED,
            post("/oauth/authorize", body, "same-origin"),
        )
        .await;
        assert_eq!(own.status, StatusCode::SEE_OTHER, "{}", own.body);
        assert!(own.location().starts_with(REDIRECT));

        // Cookie-authenticated writes elsewhere need a same-origin browser request.
        let crossed = send_from(&f.app, TRUSTED, post("/", String::new(), "cross-site")).await;
        assert_eq!(crossed.status, StatusCode::FORBIDDEN);
        let same = send_from(&f.app, TRUSTED, post("/", String::new(), "same-origin")).await;
        assert_eq!(same.status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn forward_sessions_use_the_host_prefixed_cookie_over_https() {
        let f = fwd_app_at("https://folio.example.com", |_| {}).await;
        let login = send_from(&f.app, TRUSTED, with_identity("/", "erin", &[])).await;
        assert_eq!(login.status, StatusCode::OK);
        let cookie = session_cookie(&login);
        assert!(cookie.starts_with("__Host-folio_session="), "{cookie}");
        let again = send_from(&f.app, UNTRUSTED, get_req("/", &cookie)).await;
        assert_eq!(again.status, StatusCode::OK);

        let anonymous = Request::get("/")
            .header("Cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        send_from(&f.app, TRUSTED, anonymous).await;
        assert_eq!(session_count(&f).await, 0);
    }

    #[tokio::test]
    async fn external_and_local_groups_are_separate_namespaces() {
        let f = fwd_app(|c| c.groups_header = "Remote-Groups".into()).await;
        send_from(
            &f.app,
            TRUSTED,
            with_identity("/", "carol", &[("Remote-Groups", "admins")]),
        )
        .await;
        let carol = f
            .store
            .find_user_by_external("forward", "carol")
            .await
            .unwrap()
            .unwrap();
        sqlx::query("INSERT INTO auth_groups (id, source, name, created_at) VALUES ('g-local', 'local', 'admins', 0)")
            .execute(&f.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO auth_group_members (group_id, user_id) VALUES ('g-local', $1)")
            .bind(&carol.id)
            .execute(&f.pool)
            .await
            .unwrap();
        let groups = f.store.user_groups(&carol.id).await.unwrap();
        assert_eq!(groups.len(), 2, "same name, two namespaces: {groups:?}");
        assert!(groups.iter().any(|g| g.source == GroupSource::Local));
        assert!(groups.iter().any(|g| g.source == GroupSource::External));

        // The IdP drops the group: only the external membership goes away.
        send_from(
            &f.app,
            TRUSTED,
            with_identity("/", "carol", &[("Remote-Groups", "")]),
        )
        .await;
        let groups = f.store.user_groups(&carol.id).await.unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].source, GroupSource::Local);
    }

    #[tokio::test]
    async fn a_forwarded_name_cannot_take_over_a_local_account() {
        let f = fwd_app(|_| {}).await;
        let _owner_session = setup_and_login_with(&f.app).await;
        let res = send_from(&f.app, TRUSTED, with_identity("/", "OWNER", &[])).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
        assert!(res.cookies().is_empty());
    }

    #[tokio::test]
    async fn header_names_are_configurable() {
        let f = fwd_app(|c| c.user_header = "X-Authentik-Username".into()).await;
        let ignored = send_from(&f.app, TRUSTED, with_identity("/", "dave", &[])).await;
        assert_eq!(ignored.status, StatusCode::SEE_OTHER);
        let req = Request::get("/")
            .header("Accept", "text/html")
            .header("X-Authentik-Username", "dave")
            .body(Body::empty())
            .unwrap();
        assert_eq!(send_from(&f.app, TRUSTED, req).await.status, StatusCode::OK);
    }

    #[tokio::test]
    async fn oauth_authorize_works_through_forward_auth() {
        let f = fwd_app(|_| {}).await;
        let client = register(&f.app).await;
        let uri = format!("/oauth/authorize?{}", authorize_query(&client));
        let res = send_from(&f.app, TRUSTED, with_identity(&uri, "erin", &[])).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        assert!(res
            .body
            .contains("Allow Claude to read and change your documents?"));
    }

    async fn setup_and_login_with(app: &App) -> String {
        setup_and_login(app).await
    }
}
