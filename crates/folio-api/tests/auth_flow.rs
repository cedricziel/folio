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

struct App {
    router: Router,
    _dir: tempfile::TempDir,
}

async fn app() -> App {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("t.db").display());
    let pool = create_pool(&url).await.unwrap();
    run_migrations(&pool).await.unwrap();
    let auth = AuthState::new(Arc::new(SqliteAuthStore::new(pool)), Some(BASE.into()));
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
    let page = send(app, Request::get("/setup").body(Body::empty()).unwrap()).await;
    assert_eq!(page.status, StatusCode::OK);
    let csrf = csrf_from(&page.body);
    let cookie = format!("folio_csrf={csrf}");
    let done = send(
        app,
        form_req(
            "/setup",
            &cookie,
            &format!("csrf={csrf}&username=owner&password=correct-horse-battery"),
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
        form_req("/oauth/revoke", "", &format!("token={access}")),
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

    let replay = exchange(&app, &refresh(old)).await;
    assert_eq!(replay.json()["error"], "invalid_grant");

    let new_access = second["access_token"].as_str().unwrap();
    let ok = send(
        &app,
        Request::get("/mcp")
            .header(header::AUTHORIZATION, format!("Bearer {new_access}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK);
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
        ] {
            pages = pages.route(path, get(|| async { "ok" }));
        }
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
        assert_eq!(status(&app, "/setup").await, StatusCode::OK);
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
        let auth = AuthState::new(store.clone(), Some(BASE.into())).with_forward(forward);
        let router = auth_router(auth.clone())
            .merge(
                Router::new()
                    .route("/", get(|| async { "home" }))
                    .route(
                        "/static/echo",
                        get(|headers: HeaderMap| async move {
                            let mut names: Vec<_> =
                                headers.keys().map(|k| k.as_str().to_string()).collect();
                            names.sort();
                            names.join(",")
                        }),
                    ),
            )
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
        let mut b = Request::get(uri).header("Remote-User", user);
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
            .find(|c| c.starts_with("folio_session="))
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
            with_identity("/", "bob", &[("Remote-Groups", "family|admins")]),
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
    async fn the_first_user_is_not_admin_when_a_mapping_decides() {
        let f = fwd_app(|c| {
            c.role_header = "Remote-Role".into();
            c.admin_role = Some("admin".into());
        })
        .await;
        let req = with_identity("/", "first", &[("Remote-Role", "member")]);
        send_from(&f.app, TRUSTED, req).await;
        let first = f
            .store
            .find_user_by_external("forward", "first")
            .await
            .unwrap()
            .unwrap();
        assert!(!first.is_admin);
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
