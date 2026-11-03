//! Two-factor sign-in (TOTP, recovery codes, passkeys) and the security page,
//! driven through the full router.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, HeaderMap, Request, StatusCode},
    Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use folio_api::AuthState;
use folio_auth::{crypto::hash_password, mfa, model::SOURCE_LOCAL, server::now, AuthStore, User};
use serde_json::{json, Value};
use tower::ServiceExt;
use webauthn_authenticator_rs::{
    prelude::{CreationChallengeResponse, RequestChallengeResponse, Url},
    softpasskey::SoftPasskey,
    WebauthnAuthenticator,
};

const BASE: &str = "https://folio.test";
const PASSWORD: &str = "correct horse battery";

struct Resp {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl Resp {
    fn location(&self) -> String {
        self.headers
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default()
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// One browser: a router plus its cookies.
struct Browser {
    app: Router,
    jar: HashMap<String, String>,
}

impl Browser {
    fn new(app: &Router) -> Self {
        Self {
            app: app.clone(),
            jar: HashMap::new(),
        }
    }

    async fn send(&mut self, mut req: Request<Body>) -> Resp {
        if !self.jar.is_empty() {
            let cookie = self
                .jar
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            req.headers_mut()
                .insert(header::COOKIE, cookie.parse().unwrap());
        }
        let res = self.app.clone().oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        for value in headers.get_all(header::SET_COOKIE) {
            let pair = value.to_str().unwrap().split(';').next().unwrap();
            let (name, val) = pair.split_once('=').unwrap();
            if val.is_empty() {
                self.jar.remove(name);
            } else {
                self.jar.insert(name.to_string(), val.to_string());
            }
        }
        let body = common::text(res).await;
        Resp {
            status,
            headers,
            body,
        }
    }

    async fn get(&mut self, uri: &str) -> Resp {
        self.send(Request::get(uri).body(Body::empty()).unwrap())
            .await
    }

    async fn form(&mut self, uri: &str, fields: &[(&str, &str)]) -> Resp {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        self.send(
            Request::post(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
    }

    async fn json(&mut self, uri: &str, csrf: Option<&str>, body: Option<Value>) -> Resp {
        let mut req = Request::post(uri).header(header::CONTENT_TYPE, "application/json");
        if let Some(csrf) = csrf {
            req = req.header("x-csrf", csrf);
        }
        let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
        self.send(req.body(body).unwrap()).await
    }

    fn has_session(&self) -> bool {
        self.jar.keys().any(|k| k.ends_with("folio_session"))
    }

    /// Password step of the login form; returns the response.
    async fn password_login(&mut self, username: &str, password: &str) -> Resp {
        let page = self.get("/login").await;
        let csrf = field(&page.body, "csrf");
        self.form(
            "/login",
            &[
                ("username", username),
                ("password", password),
                ("csrf", &csrf),
                ("next", "/"),
            ],
        )
        .await
    }

    async fn submit_code(&mut self, code: &str) -> Resp {
        let page = self.get("/login/code").await;
        assert_eq!(page.status, StatusCode::OK, "{}", page.body);
        let csrf = field(&page.body, "csrf");
        self.form("/login/code", &[("csrf", &csrf), ("code", code)])
            .await
    }

    async fn csrf(&mut self) -> String {
        let page = self.get("/settings/security").await;
        assert_eq!(page.status, StatusCode::OK, "{}", page.body);
        field(&page.body, "csrf")
    }
}

/// The value of the first `<input ... name="{name}" value="...">`.
fn field(body: &str, name: &str) -> String {
    let marker = format!("name=\"{name}\" value=\"");
    let start = body
        .find(&marker)
        .unwrap_or_else(|| panic!("no {name} in {body}"))
        + marker.len();
    body[start..start + body[start..].find('"').unwrap()].to_string()
}

struct World {
    app: Router,
    store: Arc<dyn AuthStore>,
    user: User,
    _h: common::Harness,
}

async fn world() -> World {
    let h = common::harness_with(Vec::new(), |store| {
        AuthState::new(store, Some(BASE.into()))
            .with_secret_key("test-secret-key-0123456789")
            .unwrap()
    })
    .await;
    let user = User {
        id: uuid::Uuid::new_v4().to_string(),
        username: "ada".into(),
        password_hash: hash_password(PASSWORD).unwrap(),
        created_at: 1,
        email: None,
        display_name: None,
        is_admin: true,
        is_owner: false,
        auth_source: SOURCE_LOCAL.into(),
        external_id: None,
    };
    h.auth_store.create_user(user.clone()).await.unwrap();
    World {
        app: h.app.clone(),
        store: h.auth_store.clone(),
        user,
        _h: h,
    }
}

/// Enrolls TOTP through the security page. Returns the signed-in browser,
/// the TOTP and the recovery codes.
async fn enroll(w: &World) -> (Browser, totp_rs::Totp, Vec<String>) {
    let mut b = Browser::new(&w.app);
    let login = b.password_login("ada", PASSWORD).await;
    assert_eq!(login.status, StatusCode::SEE_OTHER, "{}", login.body);
    assert!(b.has_session());
    let csrf = b.csrf().await;
    let start = b
        .form("/settings/security/totp/start", &[("csrf", &csrf)])
        .await;
    assert_eq!(start.status, StatusCode::SEE_OTHER);
    let page = b.get("/settings/security").await;
    assert!(page.body.contains("<svg"), "QR code is rendered inline");
    let key = page.body.split("class=\"secret-key\">").nth(1).unwrap();
    let key = &key[..key.find('<').unwrap()];
    let seed = totp_rs::Secret::try_from_base32(key)
        .unwrap()
        .as_bytes()
        .to_vec();
    let totp = mfa::totp(&seed, "ada").unwrap();

    let wrong = b
        .form(
            "/settings/security/totp/confirm",
            &[("csrf", &csrf), ("code", "000000")],
        )
        .await;
    assert!(
        wrong.location().contains("bad-code"),
        "{}",
        wrong.location()
    );
    assert!(
        !w.store
            .get_totp(&w.user.id)
            .await
            .unwrap()
            .unwrap()
            .confirmed
    );

    let code = totp.generate(now() as u64).to_string();
    let done = b
        .form(
            "/settings/security/totp/confirm",
            &[("csrf", &csrf), ("code", &code)],
        )
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let codes: Vec<String> = done
        .body
        .split("<li>")
        .skip(1)
        .map(|c| c[..c.find('<').unwrap()].to_string())
        .collect();
    assert_eq!(codes.len(), 10);
    assert!(
        w.store
            .get_totp(&w.user.id)
            .await
            .unwrap()
            .unwrap()
            .confirmed
    );
    let stored = w.store.get_totp(&w.user.id).await.unwrap().unwrap();
    assert!(
        !stored.secret_enc.contains(key),
        "the seed is encrypted at rest"
    );
    (b, totp, codes)
}

#[tokio::test]
async fn totp_login_accepts_skew_and_refuses_replay() {
    let w = world().await;
    let (_b, totp, _codes) = enroll(&w).await;

    let mut b = Browser::new(&w.app);
    let step1 = b.password_login("ada", PASSWORD).await;
    assert_eq!(step1.status, StatusCode::SEE_OTHER);
    assert_eq!(step1.location(), "/login/code");
    assert!(!b.has_session(), "the password alone does not sign in");
    let early = b.get("/").await;
    assert_eq!(
        early.status,
        StatusCode::SEE_OTHER,
        "still redirected to login"
    );

    let wrong = b.submit_code("123456").await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);

    // The code used to enroll is the step already spent.
    let enrolled_step = w
        .store
        .get_totp(&w.user.id)
        .await
        .unwrap()
        .unwrap()
        .last_step;
    let replayed = totp.generate((enrolled_step * 30) as u64).to_string();
    assert_eq!(
        b.submit_code(&replayed).await.status,
        StatusCode::UNAUTHORIZED
    );
    let past = totp.generate(((enrolled_step - 1) * 30) as u64).to_string();
    assert_eq!(b.submit_code(&past).await.status, StatusCode::UNAUTHORIZED);

    // One step ahead is within the skew.
    let ahead = totp.generate(now() as u64 + 30).to_string();
    let ok = b.submit_code(&ahead).await;
    assert_eq!(ok.status, StatusCode::SEE_OTHER, "{}", ok.body);
    assert_eq!(ok.location(), "/");
    assert!(b.has_session());

    // The same code cannot open a second login.
    let mut other = Browser::new(&w.app);
    other.password_login("ada", PASSWORD).await;
    assert_eq!(
        other.submit_code(&ahead).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn recovery_codes_work_once() {
    let w = world().await;
    let (_b, _totp, codes) = enroll(&w).await;

    let mut b = Browser::new(&w.app);
    b.password_login("ada", PASSWORD).await;
    let ok = b.submit_code(&codes[0].to_uppercase()).await;
    assert_eq!(ok.status, StatusCode::SEE_OTHER, "{}", ok.body);
    assert!(b.has_session());

    let mut again = Browser::new(&w.app);
    again.password_login("ada", PASSWORD).await;
    assert_eq!(
        again.submit_code(&codes[0]).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert!(!again.has_session());
    assert_eq!(w.store.count_recovery_codes(&w.user.id).await.unwrap(), 9);
}

#[tokio::test]
async fn wrong_codes_are_limited() {
    let w = world().await;
    let (_b, totp, _codes) = enroll(&w).await;

    let mut b = Browser::new(&w.app);
    b.password_login("ada", PASSWORD).await;
    for _ in 0..4 {
        assert_eq!(
            b.submit_code("111111").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let fifth = b.submit_code("111111").await;
    assert_eq!(fifth.status, StatusCode::TOO_MANY_REQUESTS);
    // The pending login is gone, so even the right code needs the password again.
    let page = b.get("/login/code").await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    assert_eq!(page.location(), "/login");

    // The lockout follows the account, not the pending login.
    let mut fresh = Browser::new(&w.app);
    fresh.password_login("ada", PASSWORD).await;
    let right = totp.generate(now() as u64 + 30).to_string();
    let locked = fresh.submit_code(&right).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(locked.headers.contains_key(header::RETRY_AFTER));
    assert!(!fresh.has_session());
}

#[tokio::test]
async fn pending_login_needs_the_same_browser() {
    let w = world().await;
    let (_b, totp, _codes) = enroll(&w).await;
    let mut victim = Browser::new(&w.app);
    victim.password_login("ada", PASSWORD).await;

    let mut stranger = Browser::new(&w.app);
    let page = stranger.get("/login/code").await;
    assert_eq!(page.location(), "/login");
    let code = totp.generate(now() as u64 + 30).to_string();
    let csrf = field(&stranger.get("/login").await.body, "csrf");
    let forged = stranger
        .form("/login/code", &[("csrf", &csrf), ("code", &code)])
        .await;
    assert!(!forged.status.is_success());
    assert!(!stranger.has_session());
}

#[tokio::test]
async fn disabling_and_regenerating_need_password_and_code() {
    let w = world().await;
    let (mut b, totp, codes) = enroll(&w).await;
    let csrf = b.csrf().await;
    let code = totp.generate(now() as u64 + 30).to_string();

    let bad_password = b
        .form(
            "/settings/security/totp/disable",
            &[
                ("csrf", &csrf),
                ("password", "nope nope nope"),
                ("code", &code),
            ],
        )
        .await;
    assert!(bad_password.location().contains("bad-factor"));
    let bad_code = b
        .form(
            "/settings/security/totp/disable",
            &[("csrf", &csrf), ("password", PASSWORD), ("code", "000000")],
        )
        .await;
    assert!(bad_code.location().contains("bad-factor"));
    assert!(w.store.get_totp(&w.user.id).await.unwrap().is_some());

    // Regenerating replaces the recovery codes; a recovery code can authorise it.
    let regenerated = b
        .form(
            "/settings/security/totp/recovery",
            &[("csrf", &csrf), ("password", PASSWORD), ("code", &codes[1])],
        )
        .await;
    assert_eq!(regenerated.status, StatusCode::OK, "{}", regenerated.body);
    let fresh: Vec<&str> = regenerated
        .body
        .split("<li>")
        .skip(1)
        .map(|c| &c[..14])
        .collect();
    assert_eq!(fresh.len(), 10);
    assert!(!fresh.contains(&codes[2].as_str()));
    assert!(!w
        .store
        .use_recovery_code(&w.user.id, &codes[2])
        .await
        .unwrap());

    // The TOTP steps within the skew are spent by now, so a fresh recovery code authorises it.
    let off = b
        .form(
            "/settings/security/totp/disable",
            &[("csrf", &csrf), ("password", PASSWORD), ("code", fresh[0])],
        )
        .await;
    assert!(off.location().contains("totp-off"), "{}", off.location());
    assert!(w.store.get_totp(&w.user.id).await.unwrap().is_none());
    assert_eq!(w.store.count_recovery_codes(&w.user.id).await.unwrap(), 0);

    let mut next = Browser::new(&w.app);
    let login = next.password_login("ada", PASSWORD).await;
    assert_eq!(login.location(), "/", "no code step once it is off");
}

#[tokio::test]
async fn security_forms_check_csrf() {
    let w = world().await;
    let mut b = Browser::new(&w.app);
    b.password_login("ada", PASSWORD).await;
    let res = b
        .form("/settings/security/totp/start", &[("csrf", "forged")])
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = b
        .form(
            "/settings/security/sessions/revoke-all",
            &[("csrf", "forged")],
        )
        .await;
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    assert!(b.get("/settings/security").await.status.is_success());
}

#[tokio::test]
async fn password_change_signs_out_other_devices() {
    let w = world().await;
    let mut here = Browser::new(&w.app);
    here.password_login("ada", PASSWORD).await;
    let mut there = Browser::new(&w.app);
    there.password_login("ada", PASSWORD).await;
    let csrf = here.csrf().await;
    let sessions = here.get("/settings/security").await;
    assert_eq!(sessions.body.matches("(this device)").count(), 1);

    let new = "an entirely new passphrase";
    let wrong = here
        .form(
            "/settings/security/password",
            &[
                ("csrf", &csrf),
                ("current_password", "wrong wrong wrong"),
                ("new_password", new),
                ("confirm_password", new),
            ],
        )
        .await;
    assert!(wrong.location().contains("bad-password"));
    let done = here
        .form(
            "/settings/security/password",
            &[
                ("csrf", &csrf),
                ("current_password", PASSWORD),
                ("new_password", new),
                ("confirm_password", new),
            ],
        )
        .await;
    assert!(done.location().contains("n=password"));
    assert!(here.get("/settings/security").await.status.is_success());
    assert_eq!(
        there.get("/settings/security").await.status,
        StatusCode::SEE_OTHER
    );
    let mut again = Browser::new(&w.app);
    assert_eq!(again.password_login("ada", new).await.location(), "/");
    assert_eq!(
        Browser::new(&w.app)
            .password_login("ada", PASSWORD)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn password_change_invalidates_pending_second_factor_logins() {
    let w = world().await;
    let (mut here, totp, _codes) = enroll(&w).await;
    let mut there = Browser::new(&w.app);
    there.password_login("ada", PASSWORD).await;

    let csrf = here.csrf().await;
    let code = totp.generate(now() as u64 + 30).to_string();
    let new = "an entirely new passphrase";
    let done = here
        .form(
            "/settings/security/password",
            &[
                ("csrf", &csrf),
                ("current_password", PASSWORD),
                ("new_password", new),
                ("confirm_password", new),
            ],
        )
        .await;
    assert!(done.location().contains("n=password"));

    let finished = there.submit_code(&code).await;
    assert_eq!(finished.location(), "/login");
    assert!(!there.has_session());
}

#[tokio::test]
async fn sign_out_everywhere_and_revoking_clients() {
    let w = world().await;
    let mut a = Browser::new(&w.app);
    a.password_login("ada", PASSWORD).await;
    let mut other = Browser::new(&w.app);
    other.password_login("ada", PASSWORD).await;
    let csrf = a.csrf().await;

    w.store
        .insert_client(folio_auth::OAuthClient {
            client_id: "claude".into(),
            client_name: "Claude Desktop".into(),
            redirect_uris: vec!["app:/cb".into()],
            created_at: 1,
        })
        .await
        .unwrap();
    w.store
        .insert_access_token(folio_auth::AccessToken {
            token_hash: "t".into(),
            client_id: "claude".into(),
            user_id: w.user.id.clone(),
            scopes: vec![],
            audience: "x".into(),
            expires_at: now() + 3600,
            grant_id: "g".into(),
        })
        .await
        .unwrap();
    let page = a.get("/settings/security").await;
    assert!(page.body.contains("Claude Desktop"));
    let revoked = a
        .form(
            "/settings/security/clients/claude/revoke",
            &[("csrf", &csrf)],
        )
        .await;
    assert!(revoked.location().contains("client-revoked"));
    assert!(w.store.find_access_token("t").await.unwrap().is_none());

    let out = a
        .form("/settings/security/sessions/revoke-all", &[("csrf", &csrf)])
        .await;
    assert_eq!(out.location(), "/login");
    assert!(!a.has_session());
    assert_eq!(
        other.get("/settings/security").await.status,
        StatusCode::SEE_OTHER
    );
}

// ---- passkeys ---------------------------------------------------------------

fn origin() -> Url {
    Url::parse(BASE).unwrap()
}

async fn begin_passkey_registration(
    b: &mut Browser,
    authenticator: &mut SoftPasskey,
    code: Option<&str>,
) -> Value {
    let csrf = b.csrf().await;
    let start = b
        .json(
            "/settings/security/passkeys/register/start",
            Some(&csrf),
            Some(json!({ "password": PASSWORD, "code": code.unwrap_or("") })),
        )
        .await;
    assert_eq!(start.status, StatusCode::OK, "{}", start.body);
    let mut challenge = start.json();
    assert_eq!(
        challenge["publicKey"]["authenticatorSelection"]["residentKey"],
        "required"
    );
    assert_eq!(
        challenge["publicKey"]["authenticatorSelection"]["requireResidentKey"],
        true
    );
    // SoftPasskey cannot store resident credentials; browser discovery is covered in e2e.
    challenge["publicKey"]["authenticatorSelection"]["residentKey"] = json!("discouraged");
    challenge["publicKey"]["authenticatorSelection"]["requireResidentKey"] = json!(false);
    let ccr: CreationChallengeResponse = serde_json::from_value(challenge).unwrap();
    serde_json::to_value(authenticator.do_registration(origin(), ccr).unwrap()).unwrap()
}

async fn register_passkey(
    b: &mut Browser,
    authenticator: &mut SoftPasskey,
    name: &str,
    code: Option<&str>,
) -> Resp {
    let credential = begin_passkey_registration(b, authenticator, code).await;
    let csrf = b.csrf().await;
    b.json(
        "/settings/security/passkeys/register/finish",
        Some(&csrf),
        Some(json!({ "name": name, "credential": credential })),
    )
    .await
}

/// Starts a passkey sign-in and answers it like a browser that found a
/// discoverable credential: the soft token needs the credential id up front
/// and does not return the user handle a resident key would.
async fn passkey_login(
    b: &mut Browser,
    authenticator: &mut SoftPasskey,
    credential_id: &str,
    user_id: &str,
) -> Resp {
    let start = b.json("/auth/passkey/start", None, None).await;
    assert_eq!(start.status, StatusCode::OK, "{}", start.body);
    let mut challenge = start.json();
    challenge["publicKey"]["allowCredentials"] =
        json!([{ "type": "public-key", "id": credential_id }]);
    let rcr: RequestChallengeResponse = serde_json::from_value(challenge).unwrap();
    let credential = authenticator.do_authentication(origin(), rcr).unwrap();
    let mut credential = serde_json::to_value(credential).unwrap();
    let handle = uuid::Uuid::parse_str(user_id).unwrap();
    credential["response"]["userHandle"] = json!(URL_SAFE_NO_PAD.encode(handle.as_bytes()));
    b.json(
        "/auth/passkey/finish",
        None,
        Some(json!({ "credential": credential, "next": "/" })),
    )
    .await
}

#[tokio::test]
async fn a_session_alone_cannot_register_a_passkey() {
    let w = world().await;
    let mut b = Browser::new(&w.app);
    b.password_login("ada", PASSWORD).await;
    let csrf = b.csrf().await;
    let res = b
        .json(
            "/settings/security/passkeys/register/start",
            Some(&csrf),
            Some(json!({ "password": "wrong-password" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert!(w.store.list_passkeys(&w.user.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn passkey_enrollment_requires_the_existing_second_factor() {
    let w = world().await;
    let (mut b, _, _) = enroll(&w).await;
    let csrf = b.csrf().await;
    let res = b
        .json(
            "/settings/security/passkeys/register/start",
            Some(&csrf),
            Some(json!({ "password": PASSWORD })),
        )
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert!(w.store.list_passkeys(&w.user.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn registration_is_bound_to_one_live_session_and_consumed_once() {
    let w = world().await;
    let mut first = Browser::new(&w.app);
    let mut second = Browser::new(&w.app);
    first.password_login("ada", PASSWORD).await;
    second.password_login("ada", PASSWORD).await;
    let mut authenticator = SoftPasskey::new(true);
    let credential = begin_passkey_registration(&mut first, &mut authenticator, None).await;
    let body = json!({ "name": "Laptop", "credential": credential });
    let csrf_second = second.csrf().await;
    let stolen = second
        .json(
            "/settings/security/passkeys/register/finish",
            Some(&csrf_second),
            Some(body.clone()),
        )
        .await;
    assert_eq!(stolen.status, StatusCode::BAD_REQUEST);
    let csrf_first = first.csrf().await;
    let accepted = first
        .json(
            "/settings/security/passkeys/register/finish",
            Some(&csrf_first),
            Some(body.clone()),
        )
        .await;
    assert_eq!(accepted.status, StatusCode::OK);
    let replay = first
        .json(
            "/settings/security/passkeys/register/finish",
            Some(&csrf_first),
            Some(body),
        )
        .await;
    assert_eq!(replay.status, StatusCode::BAD_REQUEST);
    assert_eq!(w.store.list_passkeys(&w.user.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_revoked_session_cannot_finish_registration() {
    let w = world().await;
    let mut b = Browser::new(&w.app);
    b.password_login("ada", PASSWORD).await;
    let mut authenticator = SoftPasskey::new(true);
    let credential = begin_passkey_registration(&mut b, &mut authenticator, None).await;
    let csrf = b.csrf().await;
    b.form("/settings/security/sessions/revoke-all", &[("csrf", &csrf)])
        .await;
    let res = b
        .json(
            "/settings/security/passkeys/register/finish",
            Some(&csrf),
            Some(json!({ "name": "Revoked", "credential": credential })),
        )
        .await;
    assert_eq!(res.status, StatusCode::SEE_OTHER);
    assert!(w.store.list_passkeys(&w.user.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn passkeys_register_sign_in_and_manage() {
    let w = world().await;
    let mut authenticator = SoftPasskey::new(true);
    let mut b = Browser::new(&w.app);
    b.password_login("ada", PASSWORD).await;

    let done = register_passkey(&mut b, &mut authenticator, "Laptop", None).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    let keys = w.store.list_passkeys(&w.user.id).await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].name, "Laptop");
    assert!(b.get("/settings/security").await.body.contains("Laptop"));

    // Username-less sign-in.
    let credential_id = keys[0].credential_id.clone();
    let mut fresh = Browser::new(&w.app);
    let login = passkey_login(&mut fresh, &mut authenticator, &credential_id, &w.user.id).await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.body);
    assert_eq!(login.json()["redirect"], "/");
    assert!(fresh.has_session());
    assert!(fresh.get("/settings/security").await.status.is_success());
    let used = &w.store.list_passkeys(&w.user.id).await.unwrap()[0];
    assert!(used.last_used_at.is_some());

    // Rename and delete.
    let csrf = b.csrf().await;
    let id = &keys[0].id;
    let renamed = b
        .form(
            &format!("/settings/security/passkeys/{id}/rename"),
            &[("csrf", &csrf), ("name", "Work laptop")],
        )
        .await;
    assert!(renamed.location().contains("passkey-renamed"));
    assert_eq!(
        w.store.list_passkeys(&w.user.id).await.unwrap()[0].name,
        "Work laptop"
    );
    let deleted = b
        .form(
            &format!("/settings/security/passkeys/{id}/delete"),
            &[("csrf", &csrf)],
        )
        .await;
    assert!(deleted.location().contains("passkey-deleted"));
    let mut gone = Browser::new(&w.app);
    let refused = passkey_login(&mut gone, &mut authenticator, &credential_id, &w.user.id).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    assert!(!gone.has_session());
}

#[tokio::test]
async fn a_passkey_satisfies_the_second_factor() {
    let w = world().await;
    let mut authenticator = SoftPasskey::new(true);
    let (mut b, _totp, codes) = enroll(&w).await;
    register_passkey(&mut b, &mut authenticator, "Phone", Some(&codes[0])).await;
    let credential_id = w.store.list_passkeys(&w.user.id).await.unwrap()[0]
        .credential_id
        .clone();

    let mut login = Browser::new(&w.app);
    let password = login.password_login("ada", PASSWORD).await;
    assert_eq!(password.location(), "/login/code");
    let done = passkey_login(&mut login, &mut authenticator, &credential_id, &w.user.id).await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.body);
    assert!(login.has_session());
    assert_eq!(
        login.get("/login/code").await.location(),
        "/login",
        "the pending step is gone"
    );
}

#[tokio::test]
async fn a_passkey_of_another_user_cannot_finish_a_pending_login() {
    let w = world().await;
    let mut authenticator = SoftPasskey::new(true);
    let mut mallory_session = Browser::new(&w.app);
    // A second local account with its own passkey.
    let mallory = User {
        id: uuid::Uuid::new_v4().to_string(),
        username: "mallory".into(),
        password_hash: hash_password(PASSWORD).unwrap(),
        ..w.user.clone()
    };
    w.store.create_user(mallory.clone()).await.unwrap();
    mallory_session.password_login("mallory", PASSWORD).await;
    register_passkey(&mut mallory_session, &mut authenticator, "Evil", None).await;
    let credential_id = w.store.list_passkeys(&mallory.id).await.unwrap()[0]
        .credential_id
        .clone();

    let (_b, _totp, _codes) = enroll(&w).await;
    let mut victim = Browser::new(&w.app);
    victim.password_login("ada", PASSWORD).await;
    let res = passkey_login(&mut victim, &mut authenticator, &credential_id, &mallory.id).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert!(!victim.has_session());
}

#[tokio::test]
async fn forged_or_missing_ceremony_state_is_refused() {
    let w = world().await;
    let mut b = Browser::new(&w.app);
    let res = b
        .json(
            "/auth/passkey/finish",
            None,
            Some(json!({ "credential": {"id": "x", "rawId": "eA", "type": "public-key",
                "response": {"authenticatorData": "eA", "clientDataJSON": "eA", "signature": "eA", "userHandle": null},
                "extensions": {}}})),
        )
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sso_accounts_get_the_security_page_without_local_enrollment() {
    let w = world().await;
    let sso = User {
        id: uuid::Uuid::new_v4().to_string(),
        username: "grace".into(),
        password_hash: String::new(),
        auth_source: format!("{}authentik", folio_auth::model::SOURCE_OIDC_PREFIX),
        external_id: Some("sub-1".into()),
        ..w.user.clone()
    };
    w.store.create_user(sso.clone()).await.unwrap();
    let session = folio_auth::crypto::random_token();
    w.store
        .create_session(folio_auth::Session {
            id_hash: folio_auth::crypto::hash_token(&session),
            user_id: sso.id.clone(),
            expires_at: now() + 600,
            created_at: now(),
            user_agent: None,
        })
        .await
        .unwrap();
    let mut b = Browser::new(&w.app);
    b.jar.insert("__Host-folio_session".into(), session);

    let page = b.get("/settings/security").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("Active sessions"), "{}", page.body);
    assert!(page.body.contains("identity provider"));
    assert!(!page.body.contains("/settings/security/totp/start"));
    assert!(!page.body.contains("passkeys/register"));

    let csrf = field(&page.body, "csrf");
    let start = b
        .form("/settings/security/totp/start", &[("csrf", &csrf)])
        .await;
    assert_ne!(start.status, StatusCode::OK);
    assert!(w.store.get_totp(&sso.id).await.unwrap().is_none());
}
