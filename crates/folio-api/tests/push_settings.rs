mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

use common::{harness_with_push, text, vault_session, Harness};
use folio_api::AuthState;
use folio_core::models::VaultRole;
use folio_core::ports::PushTransport;

const KEY: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgAosoCcYoVHPZlCWP
oShkVBX+C6hjdfOCLb99IVGjenWhRANCAAQHz9RTjiH1cvtgtneWb5gLPuVYpIpN
KAuAQloidgaveNCZtHQFTnDTR4pFwYBKElRLGQnBgDU8trF2AC/LsCzk
-----END PRIVATE KEY-----
";

async fn harness(secret: Option<&str>) -> Harness {
    harness_with_push(
        Vec::new(),
        |store| AuthState::new(store, None).with_required(true),
        Default::default(),
        secret,
    )
    .await
}

fn form(path: &str, cookie: &str, body: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::COOKIE, cookie)
        .header("host", "localhost")
        .header("origin", "http://localhost")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap()
}

fn apns_form(key: &str) -> String {
    format!(
        "transport=apns&team_id=TEAM123&key_id=KEY456&topic=com.cedricziel.folio&quiet_window_seconds=10&max_alerts_per_hour=6&private_key={}",
        urlencode(key)
    )
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn location(h: &Harness, request: Request<Body>) -> String {
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string()
}

async fn stored(h: &Harness, key: &str) -> Option<String> {
    h.state
        .settings_repo
        .get(key)
        .await
        .unwrap()
        .map(|setting| setting.value)
}

#[tokio::test]
async fn an_admin_saves_apns_credentials_sealed() {
    let h = harness(Some("an-external-secret-key-for-tests")).await;
    let admin = vault_session(&h, "admin", VaultRole::Owner, true).await;

    let to = location(&h, form("/settings/push", &admin, apns_form(KEY))).await;

    assert_eq!(to, "/settings?push=saved");
    let sealed = stored(&h, "push.apns.private_key").await.unwrap();
    assert!(sealed.starts_with("enc:v1:"));
    assert!(!sealed.contains("PRIVATE KEY"));
    let push = h.state.push.as_ref().unwrap();
    assert!(push.settings.available().await);
    let status = push.settings.status().await;
    assert_eq!(status.team_id.as_deref(), Some("TEAM123"));
    assert_eq!(status.quiet_window_seconds, 10);
    assert_eq!(status.max_alerts_per_hour, 6);

    let response = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/settings")
                .header(header::COOKIE, &admin)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = text(response).await;
    assert!(html.contains("Push notifications"));
    assert!(html.contains("A key is configured (database)"));
    assert!(
        !html.contains("PRIVATE KEY-----\nMIG"),
        "the key is never shown"
    );
}

#[tokio::test]
async fn keys_are_refused_while_the_secret_lives_in_the_database() {
    let h = harness(None).await;
    let admin = vault_session(&h, "admin", VaultRole::Owner, true).await;

    let to = location(&h, form("/settings/push", &admin, apns_form(KEY))).await;

    assert_eq!(to, "/settings?push=key_refused");
    assert_eq!(stored(&h, "push.apns.private_key").await, None);
    assert_eq!(stored(&h, "push.transport").await, None, "nothing is saved");
}

#[tokio::test]
async fn malformed_keys_are_rejected() {
    let h = harness(Some("an-external-secret-key-for-tests")).await;
    let admin = vault_session(&h, "admin", VaultRole::Owner, true).await;

    let to = location(&h, form("/settings/push", &admin, apns_form("not a key"))).await;

    assert_eq!(to, "/settings?push=bad_key");
    assert_eq!(stored(&h, "push.apns.private_key").await, None);
}

#[tokio::test]
async fn ordinary_users_cannot_change_push_settings() {
    let h = harness(Some("an-external-secret-key-for-tests")).await;
    let user = vault_session(&h, "user", VaultRole::Owner, false).await;

    for path in ["/settings/push", "/settings/push/test"] {
        let response = h
            .app
            .clone()
            .oneshot(form(path, &user, apns_form(KEY)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
    assert_eq!(stored(&h, "push.transport").await, None);
}

#[tokio::test]
async fn the_page_warns_about_a_test_endpoint() {
    let mut push = folio_infra::config::PushConfig::default();
    push.apns.base_url = Some("http://127.0.0.1:9443".into());
    let h = harness_with_push(
        Vec::new(),
        |store| AuthState::new(store, None).with_required(true),
        push,
        None,
    )
    .await;
    let admin = vault_session(&h, "admin", VaultRole::Owner, true).await;

    let response = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/settings")
                .header(header::COOKIE, &admin)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = text(response).await;
    assert!(html.contains("http://127.0.0.1:9443"));
    assert!(html.contains("not to Apple"));
}
