//! OpenID Connect login against an in-process mock identity provider.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, Request, StatusCode},
    middleware,
    response::{IntoResponse, Redirect},
    routing::{get, post},
    Form, Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use folio_api::{auth_router, enforce_auth, AuthState};
use folio_auth::{
    oidc::{OidcProvider, OidcSettings},
    AuthStore, GroupSource,
};
use folio_infra::Database;

const BASE: &str = "http://folio.test";
const CLIENT_ID: &str = "folio";

struct SigningKey {
    kid: &'static str,
    pem: &'static str,
    x: &'static str,
    y: &'static str,
}

const KEY_A: SigningKey = SigningKey {
    kid: "key-a",
    pem: "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgLGLntVQNtiQjbwUm\nA8BfV4kh3hcBlLfVG9mfZ1HJgE6hRANCAATWJqoyYVWFDsIXVQ2i86PNZsqpE3D7\nInwjTfeVijw/ckCTiB980LRuvIuOfOV+vi4vY7qLGvjzqkzItgKyyHXX\n-----END PRIVATE KEY-----\n",
    x: "1iaqMmFVhQ7CF1UNovOjzWbKqRNw-yJ8I033lYo8P3I",
    y: "QJOIH3zQtG68i4585X6-Li9juosa-POqTMi2ArLIddc",
};

const KEY_B: SigningKey = SigningKey {
    kid: "key-b",
    pem: "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgPZ6xUkiIRKDXP3K3\nZQTMiSQlQXI4ngz9N3U4YV3+5DehRANCAAQqRXB+TEudZpcUCMTLgfTJ1A2rWfl6\nktvlS7PvbOBfz3ohh/uTgj0A6485H4WxuHdH2jMHORib5Xx/nQ+PNd/O\n-----END PRIVATE KEY-----\n",
    x: "KkVwfkxLnWaXFAjEy4H0ydQNq1n5epLb5Uuz72zgX88",
    y: "eiGH-5OCPQDrjzkfhbG4d0faMwc5GJvlfH-dD481384",
};

struct Pending {
    nonce: String,
    challenge: String,
    redirect_uri: String,
}

struct MockState {
    base: String,
    published: Vec<&'static SigningKey>,
    signer: &'static SigningKey,
    claims: serde_json::Map<String, Value>,
    exp_offset: i64,
    wrong_nonce: bool,
    iss_override: Option<String>,
    end_session: bool,
    secret: Option<&'static str>,
    userinfo: Option<Value>,
    kid_override: Option<&'static str>,
    token_status: Option<StatusCode>,
    hs256_secret: Option<&'static str>,
    codes: HashMap<String, Pending>,
}

#[derive(Clone)]
struct Mock(Arc<Mutex<MockState>>);

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

impl Mock {
    async fn start() -> (Mock, String) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mock = Mock(Arc::new(Mutex::new(MockState {
            base: base.clone(),
            published: vec![&KEY_A],
            signer: &KEY_A,
            claims: json!({
                "sub": "user-1",
                "preferred_username": "alice",
                "email": "alice@example.com",
                "name": "Alice A.",
                "groups": ["family"],
            })
            .as_object()
            .unwrap()
            .clone(),
            exp_offset: 300,
            wrong_nonce: false,
            iss_override: None,
            end_session: false,
            secret: None,
            userinfo: None,
            kid_override: None,
            token_status: None,
            hs256_secret: None,
            codes: HashMap::new(),
        })));
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .route("/userinfo", get(userinfo))
            .with_state(mock.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (mock, base)
    }

    fn with<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        f(&mut self.0.lock().unwrap())
    }
}

async fn discovery(State(m): State<Mock>) -> Json<Value> {
    let s = m.0.lock().unwrap();
    let mut doc = json!({
        "issuer": s.base,
        "authorization_endpoint": format!("{}/authorize", s.base),
        "token_endpoint": format!("{}/token", s.base),
        "jwks_uri": format!("{}/jwks", s.base),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["ES256"],
        "userinfo_endpoint": format!("{}/userinfo", s.base),
    });
    if s.end_session {
        doc["end_session_endpoint"] = json!(format!("{}/logout", s.base));
    }
    Json(doc)
}

async fn userinfo(State(m): State<Mock>) -> impl IntoResponse {
    match m.0.lock().unwrap().userinfo.clone() {
        Some(info) => Json(info).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn jwks(State(m): State<Mock>) -> Json<Value> {
    let s = m.0.lock().unwrap();
    let keys: Vec<Value> = s
        .published
        .iter()
        .map(|k| json!({"kty": "EC", "crv": "P-256", "x": k.x, "y": k.y, "kid": k.kid, "alg": "ES256", "use": "sig"}))
        .collect();
    Json(json!({ "keys": keys }))
}

async fn authorize(
    State(m): State<Mock>,
    Query(q): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let code = format!("code-{}", q["state"]);
    let mut s = m.0.lock().unwrap();
    s.codes.insert(
        code.clone(),
        Pending {
            nonce: q["nonce"].clone(),
            challenge: q["code_challenge"].clone(),
            redirect_uri: q["redirect_uri"].clone(),
        },
    );
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(
        q["scope"].split(' ').filter(|s| *s == "openid").count(),
        1,
        "openid is requested once"
    );
    let mut url = url::Url::parse(&q["redirect_uri"]).unwrap();
    url.query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", &q["state"]);
    Redirect::to(url.as_str())
}

async fn token(
    State(m): State<Mock>,
    headers: HeaderMap,
    Form(f): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let mut s = m.0.lock().unwrap();
    if let Some(status) = s.token_status {
        return (status, "upstream trouble").into_response();
    }
    let denied = |why: &str| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant", "error_description": why})),
        )
            .into_response()
    };
    if let Some(secret) = s.secret {
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT_ID}:{secret}"))
        );
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            != Some(expected.as_str())
        {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "invalid_client"})),
            )
                .into_response();
        }
    }
    let Some(pending) = s
        .codes
        .remove(f.get("code").map(String::as_str).unwrap_or(""))
    else {
        return denied("unknown code");
    };
    if f.get("redirect_uri") != Some(&pending.redirect_uri) {
        return denied("redirect_uri");
    }
    let verifier = f.get("code_verifier").cloned().unwrap_or_default();
    if URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != pending.challenge {
        return denied("pkce");
    }
    let mut claims = s.claims.clone();
    claims.insert(
        "iss".into(),
        json!(s.iss_override.clone().unwrap_or(s.base.clone())),
    );
    claims.entry("aud").or_insert(json!(CLIENT_ID));
    claims.insert("iat".into(), json!(now()));
    claims.insert("exp".into(), json!(now() + s.exp_offset));
    claims.insert(
        "nonce".into(),
        json!(if s.wrong_nonce {
            "not-the-nonce".to_string()
        } else {
            pending.nonce
        }),
    );
    let (alg, key) = match s.hs256_secret {
        Some(secret) => (
            Algorithm::HS256,
            EncodingKey::from_secret(secret.as_bytes()),
        ),
        None => (
            Algorithm::ES256,
            EncodingKey::from_ec_pem(s.signer.pem.as_bytes()).unwrap(),
        ),
    };
    let mut header = Header::new(alg);
    header.kid = Some(s.kid_override.unwrap_or(s.signer.kid).to_string());
    let id_token = jsonwebtoken::encode(&header, &claims, &key).unwrap();
    Json(json!({"access_token": "opaque", "token_type": "Bearer", "id_token": id_token}))
        .into_response()
}

// ---- Folio side ------------------------------------------------------------

struct Folio {
    router: Router,
    store: Arc<dyn AuthStore>,
    mock: Mock,
    _dir: tempfile::TempDir,
}

struct Resp {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl Resp {
    fn location(&self) -> String {
        self.headers[header::LOCATION].to_str().unwrap().to_string()
    }
    fn cookie(&self, name: &str) -> Option<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|c| c.starts_with(&format!("{name}=")))
            .map(|c| c.split(';').next().unwrap().to_string())
    }
}

async fn folio_with(tweak: impl FnOnce(&mut OidcSettings)) -> Folio {
    let (mock, issuer) = Mock::start().await;
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!("sqlite:{}/t.db?mode=rwc", dir.path().display()))
        .await
        .unwrap();
    db.migrate().await.unwrap();
    let store = db.repositories().await.auth;
    let mut config = OidcSettings {
        issuer,
        client_id: CLIENT_ID.into(),
        client_secret: None,
        scopes: vec!["openid".into(), "profile".into(), "email".into()],
        display_name: "Sign in with Authentik".into(),
        allow_local_login: true,
        admin_group: Some("folio-admins".into()),
        groups_claim: "groups".into(),
        auto_create_users: true,
    };
    tweak(&mut config);
    let client = OidcProvider::new(config).unwrap();
    let auth = AuthState::new(store.clone(), Some(BASE.into())).with_oidc(client);
    let router = auth_router(auth.clone())
        .merge(Router::new().route("/", get(|| async { "home" })))
        .layer(middleware::from_fn_with_state(auth, enforce_auth));
    Folio {
        router,
        store,
        mock,
        _dir: dir,
    }
}

async fn folio() -> Folio {
    folio_with(|_| {}).await
}

async fn send(f: &Folio, req: Request<Body>) -> Resp {
    let res = f.router.clone().oneshot(req).await.unwrap();
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

/// Step one: Folio sends the browser to the provider. Returns the provider URL and state cookie.
async fn begin(f: &Folio, next: &str) -> (String, String) {
    let encoded: String = url::form_urlencoded::byte_serialize(next.as_bytes()).collect();
    let res = send(f, get_req(&format!("/auth/oidc/login?next={encoded}"), "")).await;
    assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.body);
    (
        res.location(),
        res.cookie("folio_oidc").expect("state cookie"),
    )
}

/// The provider's redirect back to Folio, as a path and query.
async fn provider_round_trip(authorize_url: &str) -> String {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let res = http.get(authorize_url).send().await.unwrap();
    assert!(res.status().is_redirection());
    let back = url::Url::parse(res.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    format!("{}?{}", back.path(), back.query().unwrap())
}

async fn login(f: &Folio, next: &str) -> Resp {
    let (url, state_cookie) = begin(f, next).await;
    let callback = provider_round_trip(&url).await;
    send(f, get_req(&callback, &state_cookie)).await
}

fn assert_rejected(res: &Resp) {
    assert!(
        res.status.is_client_error(),
        "expected a rejection, got {} {}",
        res.status,
        res.body
    );
    assert!(
        res.cookie("folio_session").is_none(),
        "no session may be issued"
    );
}

#[tokio::test]
async fn login_button_uses_the_display_name() {
    let f = folio().await;
    let page = send(&f, get_req("/login?next=%2F", "")).await;
    // No users yet and local login allowed: setup comes first.
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    f.store.create_first_user(local_owner()).await.unwrap();
    let page = send(&f, get_req("/login?next=%2Foauth%2Fauthorize%3Fa%3Db", "")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Sign in with Authentik"));
    assert!(page
        .body
        .contains("/auth/oidc/login?next=%2Foauth%2Fauthorize%3Fa%3Db"));
    assert!(
        page.body.contains("name=\"password\""),
        "local form stays while allowed"
    );
}

fn local_owner() -> folio_auth::User {
    folio_auth::User {
        id: "local-1".into(),
        username: "owner".into(),
        password_hash: "x".into(),
        created_at: 0,
        email: None,
        display_name: None,
        is_admin: true,
        is_owner: true,
        auth_source: "local".into(),
        external_id: None,
    }
}

#[tokio::test]
async fn full_login_creates_and_links_the_user() {
    let f = folio().await;
    let res = login(&f, "/oauth/authorize?client_id=x").await;
    assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.body);
    assert_eq!(res.location(), "/oauth/authorize?client_id=x");
    let session = res.cookie("folio_session").expect("session");

    let source = format!("oidc:{}", f.mock.with(|s| s.base.clone()));
    let user = f
        .store
        .find_user_by_external(&source, "user-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.username, "alice");
    assert_eq!(user.email.as_deref(), Some("alice@example.com"));
    assert_eq!(user.display_name.as_deref(), Some("Alice A."));
    assert!(user.is_admin, "first user is the owner");
    assert_eq!(
        send(&f, get_req("/", &session)).await.status,
        StatusCode::OK
    );

    // Same (issuer, sub) again: same account, fresh profile.
    f.mock
        .with(|s| s.claims.insert("email".into(), json!("new@example.com")));
    assert_eq!(login(&f, "/").await.status, StatusCode::SEE_OTHER);
    assert_eq!(f.store.count_users().await.unwrap(), 1);
    let again = f
        .store
        .find_user_by_external(&source, "user-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.id, user.id);
    assert_eq!(again.email.as_deref(), Some("new@example.com"));
    let groups = f.store.user_groups(&user.id).await.unwrap();
    assert!(groups
        .iter()
        .any(|g| g.name == "family" && g.source == GroupSource::External));
}

#[tokio::test]
async fn confidential_clients_authenticate_at_the_token_endpoint() {
    let f = folio_with(|c| c.client_secret = Some("s3cret".into())).await;
    f.mock.with(|s| s.secret = Some("s3cret"));
    assert_eq!(login(&f, "/").await.status, StatusCode::SEE_OTHER);
    let wrong = folio_with(|c| c.client_secret = Some("wrong".into())).await;
    wrong.mock.with(|s| s.secret = Some("s3cret"));
    assert_rejected(&login(&wrong, "/").await);
}

#[tokio::test]
async fn state_mismatch_is_rejected() {
    let f = folio().await;
    // A cookie from another flow does not match the returned state.
    let (url, _) = begin(&f, "/").await;
    let callback = provider_round_trip(&url).await;
    let other = begin(&f, "/").await.1;
    assert_rejected(&send(&f, get_req(&callback, &other)).await);

    // No cookie at all (login CSRF from another browser).
    let (url, _) = begin(&f, "/").await;
    let callback = provider_round_trip(&url).await;
    assert_rejected(&send(&f, get_req(&callback, "")).await);

    // A tampered state matches no stored flow.
    let (url, cookie) = begin(&f, "/").await;
    let callback = provider_round_trip(&url).await;
    let tampered = callback.replace("state=", "state=x");
    assert_rejected(&send(&f, get_req(&tampered, &cookie)).await);

    // A fresh, untampered flow still works, so none of the above was luck.
    let (url, cookie) = begin(&f, "/").await;
    let callback = provider_round_trip(&url).await;
    assert_eq!(
        send(&f, get_req(&callback, &cookie)).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn a_callback_cannot_be_replayed() {
    let f = folio().await;
    let (url, state_cookie) = begin(&f, "/").await;
    let callback = provider_round_trip(&url).await;
    assert_eq!(
        send(&f, get_req(&callback, &state_cookie)).await.status,
        StatusCode::SEE_OTHER
    );
    assert_rejected(&send(&f, get_req(&callback, &state_cookie)).await);
}

#[tokio::test]
async fn nonce_mismatch_is_rejected() {
    let f = folio().await;
    f.mock.with(|s| s.wrong_nonce = true);
    assert_rejected(&login(&f, "/").await);
    assert_eq!(f.store.count_users().await.unwrap(), 0);
}

#[tokio::test]
async fn expired_tokens_are_rejected() {
    let f = folio().await;
    f.mock.with(|s| s.exp_offset = -600);
    assert_rejected(&login(&f, "/").await);
}

#[tokio::test]
async fn tokens_for_another_audience_are_rejected() {
    let f = folio().await;
    f.mock
        .with(|s| s.claims.insert("aud".into(), json!("someone-else")));
    assert_rejected(&login(&f, "/").await);
}

#[tokio::test]
async fn tokens_from_another_issuer_are_rejected() {
    let f = folio().await;
    f.mock
        .with(|s| s.iss_override = Some("https://evil.example".into()));
    assert_rejected(&login(&f, "/").await);
}

#[tokio::test]
async fn a_token_signed_with_an_unpublished_key_is_rejected() {
    let f = folio().await;
    // Signed by key B, but the provider only publishes key A under any kid.
    f.mock.with(|s| s.signer = &KEY_B);
    assert_rejected(&login(&f, "/").await);
}

#[tokio::test]
async fn a_forged_signature_under_a_known_kid_is_rejected() {
    let f = folio().await;
    f.mock.with(|s| {
        s.signer = &KEY_B;
        s.kid_override = Some("key-a");
    });
    assert_rejected(&login(&f, "/").await);
    assert_eq!(f.store.count_users().await.unwrap(), 0);
}

#[tokio::test]
async fn key_rotation_is_picked_up_without_a_restart() {
    let f = folio().await;
    assert_eq!(login(&f, "/").await.status, StatusCode::SEE_OTHER);
    f.mock.with(|s| {
        s.published = vec![&KEY_B];
        s.signer = &KEY_B;
    });
    f.mock
        .with(|s| s.claims.insert("sub".into(), json!("user-2")));
    f.mock
        .with(|s| s.claims.insert("preferred_username".into(), json!("bob")));
    assert_eq!(login(&f, "/").await.status, StatusCode::SEE_OTHER);
    assert_eq!(f.store.count_users().await.unwrap(), 2);
}

#[tokio::test]
async fn missing_claims_are_read_from_userinfo() {
    let f = folio().await;
    f.mock.with(|s| {
        s.claims.remove("preferred_username");
        s.claims.remove("groups");
        s.userinfo = Some(
            json!({"sub": "user-1", "preferred_username": "alice", "groups": ["folio-admins"]}),
        );
    });
    assert_eq!(login(&f, "/").await.status, StatusCode::SEE_OTHER);
    let source = format!("oidc:{}", f.mock.with(|s| s.base.clone()));
    let user = f
        .store
        .find_user_by_external(&source, "user-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.username, "alice");
    let groups = f.store.user_groups(&user.id).await.unwrap();
    assert!(groups.iter().any(|g| g.name == "folio-admins"));
}

#[tokio::test]
async fn userinfo_for_another_subject_is_ignored() {
    let f = folio().await;
    f.mock.with(|s| {
        s.claims.remove("preferred_username");
        s.userinfo = Some(json!({"sub": "someone-else", "preferred_username": "mallory"}));
    });
    assert_eq!(login(&f, "/").await.status, StatusCode::SEE_OTHER);
    let source = format!("oidc:{}", f.mock.with(|s| s.base.clone()));
    let user = f
        .store
        .find_user_by_external(&source, "user-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        user.username, "alice@example.com",
        "falls back to the email"
    );
}

#[tokio::test]
async fn admin_group_maps_to_admin() {
    let f = folio().await;
    // The first user is the owner; the second shows the mapping.
    login(&f, "/").await;
    f.mock.with(|s| {
        s.claims.insert("sub".into(), json!("user-2"));
        s.claims.insert("preferred_username".into(), json!("bob"));
        s.claims.insert("groups".into(), json!(["family"]));
    });
    login(&f, "/").await;
    let source = format!("oidc:{}", f.mock.with(|s| s.base.clone()));
    assert!(
        !f.store
            .find_user_by_external(&source, "user-2")
            .await
            .unwrap()
            .unwrap()
            .is_admin
    );
    f.mock.with(|s| {
        s.claims
            .insert("groups".into(), json!(["family", "folio-admins"]))
    });
    login(&f, "/").await;
    assert!(
        f.store
            .find_user_by_external(&source, "user-2")
            .await
            .unwrap()
            .unwrap()
            .is_admin
    );
}

#[tokio::test]
async fn unknown_users_are_refused_without_auto_create() {
    let f = folio_with(|c| c.auto_create_users = false).await;
    let res = login(&f, "/").await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert_eq!(f.store.count_users().await.unwrap(), 0);
}

#[tokio::test]
async fn a_provider_username_cannot_take_over_a_local_account() {
    let f = folio().await;
    let mut owner = local_owner();
    owner.username = "alice".into();
    f.store.create_first_user(owner).await.unwrap();
    let res = login(&f, "/").await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn local_login_can_be_switched_off() {
    let f = folio_with(|c| c.allow_local_login = false).await;
    // No setup page either: the first OIDC user becomes the owner.
    let setup = send(&f, get_req("/setup", "")).await;
    assert_eq!(setup.status, StatusCode::SEE_OTHER);
    assert_eq!(setup.location(), "/login");
    let page = send(&f, get_req("/login", "")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Sign in with Authentik"));
    assert!(!page.body.contains("name=\"password\""));
    let csrf_cookie = format!("folio_csrf={}", "c".repeat(43));
    let post = Request::post("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, &csrf_cookie)
        .body(Body::from(format!(
            "csrf={}&username=owner&password=whatever-long-enough",
            "c".repeat(43)
        )))
        .unwrap();
    assert_eq!(send(&f, post).await.status, StatusCode::FORBIDDEN);

    let res = login(&f, "/").await;
    assert_eq!(res.status, StatusCode::SEE_OTHER);
    let source = format!("oidc:{}", f.mock.with(|s| s.base.clone()));
    assert!(
        f.store
            .find_user_by_external(&source, "user-1")
            .await
            .unwrap()
            .unwrap()
            .is_admin
    );
}

#[tokio::test]
async fn logout_ends_the_provider_session_when_advertised() {
    let f = folio().await;
    f.mock.with(|s| s.end_session = true);
    let res = login(&f, "/").await;
    let session = res.cookie("folio_session").unwrap();
    let csrf = "c".repeat(43);
    let logout = Request::post("/logout")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("{session}; folio_csrf={csrf}"))
        .body(Body::from(format!("csrf={csrf}")))
        .unwrap();
    let out = send(&f, logout).await;
    assert_eq!(out.status, StatusCode::SEE_OTHER);
    let target = url::Url::parse(&out.location()).unwrap();
    assert_eq!(target.path(), "/logout");
    let q: HashMap<_, _> = target.query_pairs().into_owned().collect();
    assert_eq!(q["post_logout_redirect_uri"], format!("{BASE}/login"));
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(
        q["id_token_hint"].split('.').count(),
        3,
        "the ID token is sent back as a hint"
    );
    // The Folio session is gone.
    assert_eq!(
        send(&f, get_req("/", &session)).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn logout_stays_local_without_an_end_session_endpoint() {
    let f = folio().await;
    let res = login(&f, "/").await;
    let session = res.cookie("folio_session").unwrap();
    let csrf = "c".repeat(43);
    let logout = Request::post("/logout")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("{session}; folio_csrf={csrf}"))
        .body(Body::from(format!("csrf={csrf}")))
        .unwrap();
    assert_eq!(send(&f, logout).await.location(), "/login");
}

#[tokio::test]
async fn symmetric_id_tokens_are_rejected() {
    let f = folio_with(|c| c.client_secret = Some("s3cret".into())).await;
    f.mock.with(|s| s.hs256_secret = Some("s3cret"));
    assert_rejected(&login(&f, "/").await);
    assert_eq!(f.store.count_users().await.unwrap(), 0);
}

#[tokio::test]
async fn a_failing_token_endpoint_is_a_bad_gateway() {
    let f = folio().await;
    f.mock
        .with(|s| s.token_status = Some(StatusCode::SERVICE_UNAVAILABLE));
    assert_eq!(login(&f, "/").await.status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn a_string_groups_claim_is_one_group_per_comma_item() {
    let f = folio().await;
    f.store.create_first_user(local_owner()).await.unwrap();
    f.mock.with(|s| {
        s.claims
            .insert("groups".into(), json!("Staff folio-admins, family"))
    });
    assert_eq!(login(&f, "/").await.status, StatusCode::SEE_OTHER);
    let source = format!("oidc:{}", f.mock.with(|s| s.base.clone()));
    let user = f
        .store
        .find_user_by_external(&source, "user-1")
        .await
        .unwrap()
        .unwrap();
    assert!(!user.is_admin, "a space must not split off the admin group");
    let mut names: Vec<_> = f
        .store
        .user_groups(&user.id)
        .await
        .unwrap()
        .into_iter()
        .map(|g| g.name)
        .collect();
    names.sort();
    assert_eq!(names, ["Staff folio-admins", "family"]);
}

#[tokio::test]
async fn a_failed_userinfo_call_keeps_groups_and_admin_status() {
    let f = folio().await;
    f.store.create_first_user(local_owner()).await.unwrap();
    f.mock
        .with(|s| s.claims.insert("groups".into(), json!(["folio-admins"])));
    login(&f, "/").await;
    let source = format!("oidc:{}", f.mock.with(|s| s.base.clone()));
    let user = f
        .store
        .find_user_by_external(&source, "user-1")
        .await
        .unwrap()
        .unwrap();
    assert!(user.is_admin);

    // Groups now only come from userinfo, which is down.
    f.mock.with(|s| {
        s.claims.remove("groups");
        s.userinfo = None;
    });
    assert_eq!(login(&f, "/").await.status, StatusCode::SEE_OTHER);
    let user = f
        .store
        .find_user_by_external(&source, "user-1")
        .await
        .unwrap()
        .unwrap();
    assert!(user.is_admin, "a lookup failure must not demote");
    let groups = f.store.user_groups(&user.id).await.unwrap();
    assert!(groups.iter().any(|g| g.name == "folio-admins"));
}

#[tokio::test]
async fn a_new_login_replaces_the_session_it_was_started_with() {
    let f = folio().await;
    let first = login(&f, "/").await.cookie("folio_session").unwrap();
    assert_eq!(send(&f, get_req("/", &first)).await.status, StatusCode::OK);

    let (url, binding) = begin(&f, "/").await;
    let callback = provider_round_trip(&url).await;
    let res = send(&f, get_req(&callback, &format!("{binding}; {first}"))).await;
    assert_eq!(res.status, StatusCode::SEE_OTHER);
    assert_eq!(
        send(&f, get_req("/", &first)).await.status,
        StatusCode::SEE_OTHER,
        "the old session is gone"
    );
}

#[tokio::test]
async fn the_setup_page_offers_sso_only_when_it_can_create_the_first_user() {
    let f = folio().await;
    let page = send(&f, get_req("/setup", "")).await;
    assert!(page.body.contains("Sign in with Authentik"));
    let f = folio_with(|c| c.auto_create_users = false).await;
    let page = send(&f, get_req("/setup", "")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(!page.body.contains("Sign in with Authentik"));
}

#[tokio::test]
async fn starting_logins_is_rate_limited() {
    let f = folio().await;
    let mut limited = 0;
    for _ in 0..60 {
        let res = send(&f, get_req("/auth/oidc/login", "")).await;
        match res.status {
            StatusCode::SEE_OTHER => {}
            StatusCode::TOO_MANY_REQUESTS => limited += 1,
            other => panic!("unexpected {other}"),
        }
    }
    assert!(limited > 0, "a flood of starts must be throttled");
    assert!(limited < 60, "normal use must not be");
}
