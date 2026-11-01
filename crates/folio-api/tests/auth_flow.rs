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
    let cookie = format!("folio_csrf={csrf}");
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
    assert_eq!(
        meta["token_endpoint_auth_methods_supported"],
        json!(["none"])
    );
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let prm = send(&app, Request::get(path).body(Body::empty()).unwrap())
            .await
            .json();
        assert_eq!(prm["resource"], format!("{BASE}/mcp"));
        assert_eq!(prm["authorization_servers"], json!([BASE]));
    }
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
    let prm = send(&app, spoofed("/.well-known/oauth-protected-resource"))
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

    // A forged POST with a valid CSRF pair but no or a wrong token creates nothing.
    let csrf = "c".repeat(43);
    for token in ["", "&token=wrong"] {
        let post = send(
            &app,
            form_req(
                "/setup",
                &format!("folio_csrf={csrf}"),
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
async fn login_rejects_bad_password_and_bad_csrf_and_open_redirects() {
    let app = app().await;
    setup_and_login(&app).await;
    let page = send(&app, Request::get("/login").body(Body::empty()).unwrap()).await;
    let csrf = csrf_from(&page.body);
    let cookie = format!("folio_csrf={csrf}");

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
        &format!("folio_csrf={csrf}"),
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
            &format!("folio_csrf={csrf}"),
            &format!("csrf={csrf}&token={SETUP_TOKEN}&username=owner&password={huge}"),
        ),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
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
        format!("Bearer resource_metadata=\"{BASE}/.well-known/oauth-protected-resource\"")
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
            "/.well-known/oauth-protected-resource",
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
