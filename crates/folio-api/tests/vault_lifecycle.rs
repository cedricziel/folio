mod common;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use folio_auth::{crypto::hash_token, Session, User};
use folio_core::models::{CreateDocument, Principal, UserId, VaultRole};
use serde_json::{json, Value};
use tower::ServiceExt;

use common::{harness_requiring_login, text, Harness};

struct SignedIn {
    user: UserId,
    cookie: String,
}

async fn sign_in(h: &Harness, id: &str, admin: bool) -> SignedIn {
    h.auth_store
        .create_user(User {
            id: id.into(),
            username: id.into(),
            password_hash: "!".into(),
            created_at: 1,
            email: None,
            display_name: None,
            is_admin: admin,
            is_owner: false,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    let token = format!("vault-lifecycle-{id}");
    h.auth_store
        .create_session(Session {
            id_hash: hash_token(&token),
            user_id: id.into(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    let user = UserId::from(id);
    h.state.vaults.provision_user(&user).await.unwrap();
    SignedIn {
        user,
        cookie: format!("folio_session={token}"),
    }
}

fn request(method: &str, path: &str, who: &SignedIn, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://localhost")
        .header(header::COOKIE, &who.cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn create(h: &Harness, who: &SignedIn) -> Value {
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            "/api/vaults",
            who,
            json!({"name": "Shared vault"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    serde_json::from_str(&text(response).await).unwrap()
}

#[tokio::test]
async fn only_current_owners_can_delete_or_restore_retained_vaults() {
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "lifecycle-owner", false).await;
    let viewer = sign_in(&h, "lifecycle-viewer", false).await;
    let admin = sign_in(&h, "lifecycle-admin", true).await;
    let created = create(&h, &owner).await;
    assert_eq!(created["created_by"], json!(owner.user));
    assert_eq!(created["updated_by"], json!(owner.user));
    let vault = created["id"].as_str().unwrap();
    let vault_id = vault.parse().unwrap();
    h.members
        .grant(
            vault_id,
            &Principal::User(viewer.user.clone()),
            VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();
    let doc = h
        .state
        .document_repo
        .create(
            vault_id,
            CreateDocument {
                title: "Retained private document".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let path = format!("/api/vaults/{vault}");
    let rejected = h
        .app
        .clone()
        .oneshot(request("DELETE", &path, &viewer, json!({})))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    let hidden = h
        .app
        .clone()
        .oneshot(request("DELETE", &path, &admin, json!({})))
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
    let deleted = h
        .app
        .clone()
        .oneshot(request("DELETE", &path, &owner, json!({})))
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let hidden_doc = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/vaults/{vault}/documents"),
            &owner,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(hidden_doc.status(), StatusCode::NOT_FOUND);
    let history_path = format!("{path}/history");
    let history = h
        .app
        .clone()
        .oneshot(request("GET", &history_path, &owner, json!({})))
        .await
        .unwrap();
    assert_eq!(history.status(), StatusCode::OK);
    assert_eq!(
        history.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert!(text(history).await.contains("vault.deleted"));
    let history = h
        .app
        .clone()
        .oneshot(request("GET", &history_path, &viewer, json!({})))
        .await
        .unwrap();
    assert_eq!(history.status(), StatusCode::NOT_FOUND);
    let restore = format!("{path}/restore");
    let denied = h
        .app
        .clone()
        .oneshot(request("POST", &restore, &viewer, json!({})))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::NOT_FOUND);
    let hidden = h
        .app
        .clone()
        .oneshot(request("POST", &restore, &admin, json!({})))
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
    let restored = h
        .app
        .clone()
        .oneshot(request("POST", &restore, &owner, json!({})))
        .await
        .unwrap();
    assert_eq!(restored.status(), StatusCode::OK);
    assert!(h
        .state
        .document_repo
        .find_by_id(vault_id, doc.id)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn vault_write_bodies_cannot_override_server_lifecycle_metadata() {
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "lifecycle-spoof-owner", false).await;
    let forged = h.app.clone().oneshot(request("POST", "/api/vaults", &owner, json!({
        "name": "Forged", "created_by": "someone-else", "updated_by": "someone-else", "deleted_at": "2020-01-01T00:00:00Z"
    }))).await.unwrap();
    assert_eq!(forged.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let created = create(&h, &owner).await;
    let path = format!("/api/vaults/{}/members", created["id"].as_str().unwrap());
    let forged = h.app.clone().oneshot(request("POST", &path, &owner, json!({
        "principal_type": "user", "principal_id": owner.user, "role": "owner", "granted_by": "someone-else"
    }))).await.unwrap();
    assert_eq!(forged.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn vault_management_page_exposes_owner_controls_and_deleted_vault_recovery() {
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "lifecycle-page-owner", false).await;
    let created = create(&h, &owner).await;
    let page = h
        .app
        .clone()
        .oneshot(request("GET", "/settings/vaults", &owner, json!({})))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = text(page).await;
    assert!(html.contains("Create vault"));
    assert!(html.contains("Shared vault"));
    assert!(html.contains("Add member"));
    let deleted = h
        .app
        .clone()
        .oneshot(request(
            "DELETE",
            &format!("/api/vaults/{}", created["id"].as_str().unwrap()),
            &owner,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let page = h
        .app
        .clone()
        .oneshot(request("GET", "/settings/vaults", &owner, json!({})))
        .await
        .unwrap();
    let html = text(page).await;
    assert!(html.contains("Deleted vaults"));
    assert!(html.contains("Restore vault"));
}

#[tokio::test]
async fn owners_can_recover_without_an_active_default_after_losing_shared_access() {
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "recovery-no-default", false).await;
    let personal = h
        .state
        .vaults
        .default_vault(Some(&owner.user))
        .await
        .unwrap();
    let shared = h
        .vault_repository
        .create("Temporary shared default", None)
        .await
        .unwrap()
        .id;
    h.members
        .grant(
            shared,
            &Principal::User(owner.user.clone()),
            VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();
    h.vault_repository
        .set_default(&owner.user, shared)
        .await
        .unwrap();
    h.vault_repository.delete(personal).await.unwrap();
    h.members
        .revoke(shared, &Principal::User(owner.user.clone()))
        .await
        .unwrap();
    assert!(h
        .vault_repository
        .list_for_user(&owner.user)
        .await
        .unwrap()
        .is_empty());

    let page = h
        .app
        .clone()
        .oneshot(request("GET", "/settings/vaults", &owner, json!({})))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(page.headers()[header::CACHE_CONTROL], "private, no-store");
    let page = text(page).await;
    assert!(page.contains("Restore vault"));
    assert!(!page.contains("data-vault-id="));
    let home = h
        .app
        .clone()
        .oneshot(request("GET", "/", &owner, json!({})))
        .await
        .unwrap();
    assert_eq!(home.status(), StatusCode::SEE_OTHER);
    assert_eq!(home.headers()[header::LOCATION], "/settings/vaults");
    let discovery = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            "/api/vaults?include_deleted=true",
            &owner,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(discovery.status(), StatusCode::OK);
    let discovery: Value = serde_json::from_str(&text(discovery).await).unwrap();
    assert_eq!(discovery["default_vault_id"], json!(null));
    assert_eq!(discovery["vaults"], json!([]));
    assert_eq!(discovery["deleted_vaults"][0]["id"], personal.to_string());
    let blocked = h
        .app
        .clone()
        .oneshot(request("GET", "/api/documents", &owner, json!({})))
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::NOT_FOUND);
    let restored = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/api/vaults/{personal}/restore"),
            &owner,
            json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(restored.status(), StatusCode::OK);
    assert_eq!(
        h.state
            .vaults
            .default_vault(Some(&owner.user))
            .await
            .unwrap(),
        personal
    );
}

#[tokio::test]
async fn only_owners_can_list_vault_members() {
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "members-owner", false).await;
    let viewer = sign_in(&h, "members-viewer", false).await;
    let created = create(&h, &owner).await;
    let vault = created["id"].as_str().unwrap();
    h.members
        .grant(
            vault.parse().unwrap(),
            &Principal::User(viewer.user.clone()),
            VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();
    let path = format!("/api/vaults/{vault}/members");

    let listed = h
        .app
        .clone()
        .oneshot(request("GET", &path, &owner, json!({})))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let hidden = h
        .app
        .clone()
        .oneshot(request("GET", &path, &viewer, json!({})))
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
}
