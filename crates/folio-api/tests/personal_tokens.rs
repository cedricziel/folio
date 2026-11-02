mod common;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::{harness, harness_requiring_login, text, Harness};
use folio_auth::{crypto::hash_token, Session, User};
use folio_core::models::UserId;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn account(h: &Harness, name: &str) -> String {
    h.auth_store
        .create_user(User {
            id: name.into(),
            username: name.into(),
            password_hash: "!".into(),
            created_at: 1,
            email: None,
            display_name: None,
            is_admin: false,
            is_owner: false,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    h.auth_store
        .create_session(Session {
            id_hash: hash_token(name),
            user_id: name.into(),
            expires_at: i64::MAX,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    h.state
        .vaults
        .provision_user(&UserId(name.into()))
        .await
        .unwrap();
    format!("folio_session={name}")
}

fn req(method: &str, uri: &str, cookie: &str, bearer: &str, body: Value) -> Request<Body> {
    let mut r = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if !cookie.is_empty() {
        r = r.header(header::COOKIE, cookie);
    }
    if !bearer.is_empty() {
        r = r.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    r.body(Body::from(body.to_string())).unwrap()
}

fn creation() -> Value {
    json!({"name":" Script ","audience":"api","scopes":["documents:read"],"expires_in_days":30})
}

async fn create(h: &Harness, cookie: &str, body: Value) -> Value {
    let response = h
        .app
        .clone()
        .oneshot(req("POST", "/api/auth/tokens", cookie, "", body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    serde_json::from_str(&text(response).await).unwrap()
}

#[tokio::test]
async fn creates_once_lists_metadata_and_revokes_only_owned_tokens() {
    let h = harness_requiring_login().await;
    let alice = account(&h, "alice").await;
    let bob = account(&h, "bob").await;
    let created = create(&h, &alice, creation()).await;
    assert_eq!(created["name"], "Script");
    let token = created["token"].as_str().unwrap();
    assert!(token.starts_with("folio_pat_") && token.len() >= 53);
    let listed = h
        .app
        .clone()
        .oneshot(req("GET", "/api/auth/tokens", &alice, "", Value::Null))
        .await
        .unwrap();
    assert_eq!(listed.headers()[header::CACHE_CONTROL], "no-store");
    let listed = text(listed).await;
    assert!(!listed.contains(token));
    assert!(!listed.contains("token_hash"));
    let metadata: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(metadata[0]["id"], created["id"]);
    let path = format!("/api/auth/tokens/{}", created["id"].as_str().unwrap());
    let other = h
        .app
        .clone()
        .oneshot(req("DELETE", &path, &bob, "", Value::Null))
        .await
        .unwrap();
    assert_eq!(other.status(), StatusCode::NOT_FOUND);
    let revoke = h
        .app
        .clone()
        .oneshot(req("DELETE", &path, &alice, "", Value::Null))
        .await
        .unwrap();
    assert_eq!(revoke.status(), StatusCode::NO_CONTENT);
    let denied = h
        .app
        .clone()
        .oneshot(req("GET", "/api/documents", "", token, Value::Null))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn token_management_requires_a_session_even_in_anonymous_mode() {
    let h = harness().await;
    let response = h
        .app
        .clone()
        .oneshot(req("POST", "/api/auth/tokens", "", "", creation()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let alice = account(&h, "alice").await;
    let token = create(&h, &alice, creation()).await;
    let response = h
        .app
        .clone()
        .oneshot(req(
            "GET",
            "/api/auth/tokens",
            &alice,
            token["token"].as_str().unwrap(),
            Value::Null,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn token_creation_rejects_cross_origin_and_invalid_permissions() {
    let h = harness_requiring_login().await;
    let alice = account(&h, "alice").await;
    let mut crossed = req("POST", "/api/auth/tokens", &alice, "", creation());
    crossed
        .headers_mut()
        .insert(header::ORIGIN, "https://evil.test".parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(crossed).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    for body in [
        json!({"name":" ","audience":"api","scopes":["documents:read"],"expires_in_days":30}),
        json!({"name":"a","audience":"admin","scopes":["documents:read"],"expires_in_days":30}),
        json!({"name":"a","audience":"api","scopes":[],"expires_in_days":30}),
        json!({"name":"a","audience":"api","scopes":["*"],"expires_in_days":30}),
        json!({"name":"a","audience":"api","scopes":["documents:read"],"expires_in_days":366}),
    ] {
        assert_eq!(
            h.app
                .clone()
                .oneshot(req("POST", "/api/auth/tokens", &alice, "", body))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn explicit_credentials_override_cookies_and_are_scoped_in_optional_auth() {
    let h = harness().await;
    let alice = account(&h, "alice").await;
    let token = create(&h, &alice, creation()).await;
    let token = token["token"].as_str().unwrap();
    assert_eq!(
        h.app
            .clone()
            .oneshot(req("GET", "/api/documents", &alice, "invalid", Value::Null))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(req(
                "POST",
                "/api/documents",
                &alice,
                token,
                json!({"title":"denied"})
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(req(
                "POST",
                "/api/documents/search",
                "",
                token,
                json!({"query":"invoice"})
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(req("GET", "/api/sources", &alice, token, Value::Null))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn credential_management_does_not_require_an_accessible_vault() {
    let h = harness_requiring_login().await;
    let cookie = account(&h, "isolated").await;
    let user = UserId::from("isolated");
    let vault = h.state.vaults.default_vault(Some(&user)).await.unwrap();
    let token = create(&h, &cookie, creation()).await;
    account(&h, "other-owner").await;
    h.members
        .grant(
            vault,
            &folio_core::models::Principal::User("other-owner".into()),
            folio_core::models::VaultRole::Owner,
            None,
        )
        .await
        .unwrap();
    h.members
        .revoke(vault, &folio_core::models::Principal::User(user))
        .await
        .unwrap();
    let response = h
        .app
        .clone()
        .oneshot(req("GET", "/api/auth/tokens", &cookie, "", Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let path = format!("/api/auth/tokens/{}", token["id"].as_str().unwrap());
    let response = h
        .app
        .clone()
        .oneshot(req("DELETE", &path, &cookie, "", Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}
