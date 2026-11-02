mod common;
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::{harness_requiring_login, Harness};
use folio_auth::{crypto::hash_token, Session, User};
use folio_core::models::{UserId, VaultId};
use tower::ServiceExt;
struct SignedIn {
    user: UserId,
    vault: VaultId,
    cookie: String,
}

async fn sign_in(h: &Harness, name: &str) -> SignedIn {
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
    let token = format!("vault-test-session-{name}");
    h.auth_store
        .create_session(Session {
            id_hash: hash_token(&token),
            user_id: name.into(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            created_at: 1,
            user_agent: Some("vault regression".into()),
        })
        .await
        .unwrap();
    let user = UserId::from(name);
    let vault = h.state.vaults.provision_user(&user).await.unwrap();
    SignedIn {
        user,
        vault,
        cookie: format!("folio_session={token}"),
    }
}

fn request(method: &str, path: &str, signed_in: &SignedIn, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://localhost")
        .header(header::COOKIE, &signed_in.cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .unwrap()
}

#[tokio::test]
async fn rename_preserves_query_and_audit_identity_until_deleted() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let collection = format!("/api/vaults/{}/saved-views", alice.vault);
    let response = h.app.clone().oneshot(request(
        "POST", &collection, &alice,
        Body::from(r#"{"name":"Invoices","query":{"version":1,"filters":{"created_after":"2024-01-01T00:00:00.123456789Z"}},"presentation":"grid"}"#),
    )).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let original: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    let path = format!("{}/{}", collection, original["id"].as_str().unwrap());
    let response = h
        .app
        .clone()
        .oneshot(request(
            "PATCH",
            &path,
            &alice,
            Body::from(r#"{"name":"Tax invoices"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let renamed: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(renamed["name"], "Tax invoices");
    assert_eq!(renamed["query"], original["query"]);
    assert_eq!(renamed["presentation"], "grid");
    assert_eq!(renamed["owner_id"], "alice");
    assert_eq!(renamed["created_by"], "alice");
    assert_eq!(renamed["updated_by"], "alice");
    assert_eq!(renamed["created_at"], original["created_at"]);
    for body in [r#"{"query":null}"#, r#"{"owner_id":"someone-else"}"#] {
        let response = h
            .app
            .clone()
            .oneshot(request("PATCH", &path, &alice, Body::from(body)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let retained: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(retained, renamed);
    let response = h
        .app
        .clone()
        .oneshot(request("DELETE", &path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn private_saved_view_is_hidden_from_another_member_and_vault() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let bob = sign_in(&h, "bob").await;
    let path = format!("/api/vaults/{}/saved-views", alice.vault);
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &path,
            &alice,
            Body::from(r#"{"name":"Invoices","query":{"version":1}}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(body["owner_id"], "alice");
    assert_eq!(body["created_by"], "alice");
    let id = body["id"].as_str().unwrap();
    let response = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/vaults/{}/saved-views/{id}", bob.vault),
            &bob,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &format!("{path}/{id}"), &bob, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn viewer_can_save_privately_but_cannot_publish() {
    use folio_core::models::{Principal, VaultRole};
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "owner").await;
    let viewer = sign_in(&h, "viewer").await;
    h.members
        .grant(
            owner.vault,
            &Principal::User(viewer.user.clone()),
            VaultRole::Viewer,
            Some(&owner.user),
        )
        .await
        .unwrap();
    let path = format!("/api/vaults/{}/saved-views", owner.vault);
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &path,
            &viewer,
            Body::from(r#"{"name":"Private","query":{"version":1}}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &path,
            &viewer,
            Body::from(r#"{"name":"Shared","visibility":"shared","query":{"version":1}}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn cross_vault_filter_is_rejected_without_saving() {
    use folio_core::models::CreateTag;
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let bob = sign_in(&h, "bob").await;
    let tag = h
        .state
        .tag_repo
        .create(
            bob.vault,
            CreateTag {
                name: "Secret".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let path = format!("/api/vaults/{}/saved-views", alice.vault);
    let input =
        serde_json::json!({"name":"Invalid", "query":{"version":1,"filters":{"tag_ids":[tag.id]}}});
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &path,
            &alice,
            Body::from(input.to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(common::text(response).await, "[]");
}
