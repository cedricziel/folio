mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use common::{harness_requiring_login, harness_with_push, text, vault_session, Harness};
use folio_api::AuthState;
use folio_core::models::{UserId, VaultRole};

const TOKEN: &str = "app-access-token";

/// `name` signed in to the app through OAuth, as an editor of the harness vault
async fn app_user(h: &Harness, name: &str, grant: &str) -> String {
    vault_session(h, name, VaultRole::Editor, false).await;
    if h.auth_store
        .find_client("folio-app")
        .await
        .unwrap()
        .is_none()
    {
        h.auth_store
            .insert_client(folio_auth::OAuthClient {
                client_id: "folio-app".into(),
                client_name: "Folio".into(),
                redirect_uris: vec!["folio://oauth".into()],
                created_at: 1,
            })
            .await
            .unwrap();
    }
    let token = format!("{TOKEN}-{name}");
    h.auth_store
        .insert_access_token(folio_auth::AccessToken {
            token_hash: folio_auth::crypto::hash_token(&token),
            client_id: "folio-app".into(),
            user_id: name.into(),
            scopes: vec!["documents:read".into(), "documents:write".into()],
            audience: "http://localhost/api".into(),
            expires_at: i64::MAX / 2,
            grant_id: grant.into(),
        })
        .await
        .unwrap();
    format!("Bearer {token}")
}

fn request(method: &str, uri: &str, auth: &str, body: Value) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    let builder = if auth.starts_with("Bearer ") {
        builder.header(header::AUTHORIZATION, auth)
    } else {
        builder.header(header::COOKIE, auth)
    };
    let body = if body.is_null() {
        Body::empty()
    } else {
        Body::from(body.to_string())
    };
    builder.body(body).unwrap()
}

async fn send(h: &Harness, request: Request<Body>) -> (StatusCode, Value) {
    let response = h.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = text(response).await;
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

fn registration(token: &str) -> Value {
    json!({
        "platform": "ios",
        "environment": "production",
        "locale": "de-DE",
        "alert_token": token,
    })
}

#[tokio::test]
async fn an_app_registers_and_refreshes_its_device() {
    let h = harness_requiring_login().await;
    let alice = app_user(&h, "alice", "grant-a").await;

    let (status, body) = send(
        &h,
        request(
            "PUT",
            "/api/devices/phone-1",
            &alice,
            registration(&"ab".repeat(32)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["push_available"], false);
    assert_eq!(body["device"]["locale"], "de-DE");
    assert_eq!(body["device"]["has_alert_token"], true);
    assert_eq!(
        body["device"]["preferences"],
        json!({"processed": false, "needs_review": true, "failed": true,
               "scope": "mine", "vaults": "all", "show_titles": false})
    );
    assert!(
        body.to_string().find(&"ab".repeat(32)).is_none(),
        "tokens are never returned"
    );

    let (status, _) = send(
        &h,
        request(
            "PUT",
            "/api/devices/phone-1",
            &alice,
            registration(&"cd".repeat(32)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let device = h
        .state
        .push
        .as_ref()
        .unwrap()
        .devices
        .find(&UserId::from("alice"), "phone-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(device.grant_id, "grant-a");
    assert_eq!(device.client_id, "folio-app");
    assert_eq!(device.alert_token.unwrap().value, "cd".repeat(32));
}

#[tokio::test]
async fn only_oauth_apps_may_register() {
    let h = harness_requiring_login().await;
    let cookie = vault_session(&h, "carol", VaultRole::Editor, false).await;

    let (status, _) = send(
        &h,
        request(
            "PUT",
            "/api/devices/phone-1",
            &cookie,
            registration(&"ab".repeat(32)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    h.auth_store
        .insert_personal_token(folio_auth::PersonalToken {
            id: "pat-1".into(),
            user_id: "carol".into(),
            name: "script".into(),
            token_hash: folio_auth::crypto::hash_token("folio_pat_personal"),
            prefix: "folio_pat".into(),
            scopes: vec!["documents:read".into(), "documents:write".into()],
            audience: "http://localhost/api".into(),
            created_at: 1,
            expires_at: i64::MAX / 2,
            revoked_at: None,
        })
        .await
        .unwrap();
    let (status, _) = send(
        &h,
        request(
            "PUT",
            "/api/devices/phone-1",
            "Bearer folio_pat_personal",
            registration(&"ab".repeat(32)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        h.state
            .push
            .as_ref()
            .unwrap()
            .devices
            .count_devices()
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn invalid_registrations_are_unprocessable() {
    let h = harness_requiring_login().await;
    let alice = app_user(&h, "alice", "grant-a").await;
    let token = "ab".repeat(32);
    let cases = [
        (
            "phone-1",
            json!({"platform": "android", "environment": "production", "locale": "en"}),
        ),
        (
            "phone-1",
            json!({"platform": "ios", "environment": "staging", "locale": "en"}),
        ),
        (
            "phone-1",
            json!({"platform": "ios", "environment": "production", "locale": "en", "alert_token": "not hex"}),
        ),
        (
            "phone-1",
            json!({"platform": "ios", "environment": "production", "locale": "e n"}),
        ),
        ("bad id!", registration(&token)),
        (
            "phone-1",
            json!({"platform": "ios", "environment": "production", "locale": "en",
            "preferences": {"vaults": [uuid::Uuid::new_v4()]}}),
        ),
    ];
    for (id, body) in cases {
        let uri = format!("/api/devices/{}", id.replace(' ', "%20"));
        let (status, _) = send(&h, request("PUT", &uri, &alice, body.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{id} {body}");
    }
}

#[tokio::test]
async fn people_list_and_delete_only_their_own_devices() {
    let h = harness_requiring_login().await;
    let alice = app_user(&h, "alice", "grant-a").await;
    let bob = app_user(&h, "bob", "grant-b").await;
    send(
        &h,
        request(
            "PUT",
            "/api/devices/phone",
            &alice,
            registration(&"ab".repeat(32)),
        ),
    )
    .await;
    send(
        &h,
        request(
            "PUT",
            "/api/devices/mac",
            &alice,
            registration(&"cd".repeat(32)),
        ),
    )
    .await;
    send(
        &h,
        request(
            "PUT",
            "/api/devices/ipad",
            &bob,
            registration(&"ef".repeat(32)),
        ),
    )
    .await;

    let (status, body) = send(&h, request("GET", "/api/devices", &alice, Value::Null)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["devices"].as_array().unwrap().len(), 2);
    assert!(!body.to_string().contains(&"ab".repeat(32)));

    for _ in 0..2 {
        let (status, _) = send(
            &h,
            request("DELETE", "/api/devices/ipad", &alice, Value::Null),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    let (_, body) = send(&h, request("GET", "/api/devices", &bob, Value::Null)).await;
    assert_eq!(
        body["devices"].as_array().unwrap().len(),
        1,
        "bob's device is untouched"
    );

    let (status, _) = send(
        &h,
        request("DELETE", "/api/devices/phone", &alice, Value::Null),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = send(&h, request("GET", "/api/devices", &alice, Value::Null)).await;
    assert_eq!(body["devices"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn signing_out_deletes_the_device() {
    let h = harness_requiring_login().await;
    let alice = app_user(&h, "alice", "grant-a").await;
    send(
        &h,
        request(
            "PUT",
            "/api/devices/phone",
            &alice,
            registration(&"ab".repeat(32)),
        ),
    )
    .await;

    h.auth_store.revoke_grant("grant-a").await.unwrap();

    let devices = &h.state.push.as_ref().unwrap().devices;
    assert!(devices
        .find(&UserId::from("alice"), "phone")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_notifications_need_push() {
    let h = harness_requiring_login().await;
    let alice = app_user(&h, "alice", "grant-a").await;
    send(
        &h,
        request(
            "PUT",
            "/api/devices/phone",
            &alice,
            registration(&"ab".repeat(32)),
        ),
    )
    .await;

    let (status, _) = send(
        &h,
        request("POST", "/api/devices/phone/test", &alice, Value::Null),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = send(
        &h,
        request("POST", "/api/devices/unknown/test", &alice, Value::Null),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_notifications_are_accepted_once_a_minute() {
    let mut push = folio_infra::config::PushConfig {
        transport: "apns".into(),
        ..Default::default()
    };
    push.apns.team_id = Some("TEAM".into());
    push.apns.key_id = Some("KEY".into());
    push.apns.private_key = Some(TEST_KEY.into());
    // Nothing listens there; the test only checks what the API answers.
    push.apns.base_url = Some("http://127.0.0.1:9".into());
    let h = harness_with_push(
        Vec::new(),
        |store| AuthState::new(store, None).with_required(true),
        push,
        None,
    )
    .await;
    let alice = app_user(&h, "alice", "grant-a").await;
    let (_, body) = send(
        &h,
        request(
            "PUT",
            "/api/devices/phone",
            &alice,
            registration(&"ab".repeat(32)),
        ),
    )
    .await;
    assert_eq!(body["push_available"], true);

    let (status, _) = send(
        &h,
        request("POST", "/api/devices/phone/test", &alice, Value::Null),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, _) = send(
        &h,
        request("POST", "/api/devices/phone/test", &alice, Value::Null),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgAosoCcYoVHPZlCWP
oShkVBX+C6hjdfOCLb99IVGjenWhRANCAAQHz9RTjiH1cvtgtneWb5gLPuVYpIpN
KAuAQloidgaveNCZtHQFTnDTR4pFwYBKElRLGQnBgDU8trF2AC/LsCzk
-----END PRIVATE KEY-----
";

#[tokio::test]
async fn long_vault_lists_are_refused_before_any_lookup() {
    let h = harness_requiring_login().await;
    let alice = app_user(&h, "alice", "grant-a").await;
    let vaults: Vec<_> = (0..101).map(|_| uuid::Uuid::new_v4()).collect();
    let body = json!({"platform": "ios", "environment": "production", "locale": "en",
        "preferences": {"vaults": vaults}});

    let (status, body) = send(&h, request("PUT", "/api/devices/phone-1", &alice, body)).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.to_string().contains("Too many vaults"), "{body}");
}
