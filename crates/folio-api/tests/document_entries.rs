mod common;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::Response,
};
use folio_auth::{crypto::hash_token, Session, User};
use folio_core::models::{CreateDocument, Principal, UserId, VaultId, VaultRole};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

async fn member(h: &common::Harness, name: &str, vault: VaultId, role: VaultRole) {
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
            expires_at: i64::MAX / 2,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    h.members
        .grant(vault, &Principal::User(UserId(name.into())), role, None)
        .await
        .unwrap();
}

async fn call(
    h: &common::Harness,
    actor: &str,
    method: &str,
    path: &str,
    key: Option<Uuid>,
    body: Value,
) -> Response {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://localhost")
        .header(header::COOKIE, format!("folio_session={actor}"))
        .header("x-folio-vault", h.vault.to_string())
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(key) = key {
        request = request.header("Idempotency-Key", key.to_string());
    }
    h.app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn setup() -> (common::Harness, String) {
    let h = common::harness_requiring_login().await;
    member(&h, "author", h.vault, VaultRole::Contributor).await;
    member(&h, "editor", h.vault, VaultRole::Editor).await;
    member(&h, "viewer", h.vault, VaultRole::Viewer).await;
    let document = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Other person's document".into(),
                uploaded_by: Some(UserId("editor".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let path = format!("/api/documents/{}/entries", document.id);
    (h, path)
}

#[tokio::test]
async fn contributor_can_annotate_another_upload_and_retry_without_duplicates() {
    let (h, path) = setup().await;
    let key = Uuid::new_v4();
    let body = json!({"kind":"note", "body":" original "});
    let created = call(&h, "author", "POST", &path, Some(key), body.clone()).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let entry: Value = serde_json::from_str(&common::text(created).await).unwrap();
    assert_eq!(entry["created_by"], "author");
    assert_eq!(entry["can_edit"], true);
    let replay = call(&h, "author", "POST", &path, Some(key), body).await;
    assert_eq!(replay.status(), StatusCode::CREATED);
    assert_eq!(
        serde_json::from_str::<Value>(&common::text(replay).await).unwrap(),
        entry
    );
    let conflict = call(
        &h,
        "author",
        "POST",
        &path,
        Some(key),
        json!({"kind":"note","body":"different"}),
    )
    .await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let page = call(
        &h,
        "viewer",
        "GET",
        &format!("{path}?kind=note"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(page.status(), StatusCode::OK);
    let page: Value = serde_json::from_str(&common::text(page).await).unwrap();
    assert_eq!(page["entries"].as_array().unwrap().len(), 1);
    assert_eq!(page["entries"][0]["can_edit"], false);
    assert_eq!(page["can_create"], false);
}

#[tokio::test]
async fn entries_enforce_authorship_moderation_revocation_and_deleted_retries() {
    let (h, path) = setup().await;
    let key = Uuid::new_v4();
    let body = json!({"kind":"comment","body":"discussion"});
    let created = call(&h, "author", "POST", &path, Some(key), body.clone()).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let entry: Value = serde_json::from_str(&common::text(created).await).unwrap();
    let item = format!("{path}/{}", entry["id"].as_str().unwrap());
    assert_eq!(
        call(
            &h,
            "editor",
            "PATCH",
            &item,
            None,
            json!({"body":"rewrite"})
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &h,
            "viewer",
            "POST",
            &path,
            Some(Uuid::new_v4()),
            body.clone()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &h,
            "author",
            "PATCH",
            &item,
            None,
            json!({"body":"changed"})
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        call(&h, "editor", "DELETE", &item, None, Value::Null)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&h, "author", "POST", &path, Some(key), body.clone())
            .await
            .status(),
        StatusCode::GONE
    );
    h.members
        .revoke(h.vault, &Principal::User(UserId("author".into())))
        .await
        .unwrap();
    assert_eq!(
        call(&h, "author", "POST", &path, Some(key), body)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn creation_rejects_missing_keys_spoofed_authors_and_invalid_cursors() {
    let (h, path) = setup().await;
    assert_eq!(
        call(
            &h,
            "author",
            "POST",
            &path,
            None,
            json!({"kind":"note","body":"text"})
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &h,
            "author",
            "POST",
            &path,
            Some(Uuid::nil()),
            json!({"kind":"note","body":"text"})
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &h,
            "author",
            "POST",
            &path,
            Some(Uuid::new_v4()),
            json!({"kind":"note","body":"text","created_by":"editor"})
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            &h,
            "author",
            "GET",
            &format!("{path}?kind=note&cursor=invalid"),
            None,
            Value::Null
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn pagination_cannot_cross_kind_document_or_vault_boundaries() {
    let (h, path) = setup().await;
    let first = call(
        &h,
        "author",
        "POST",
        &path,
        Some(Uuid::new_v4()),
        json!({"kind":"note","body":"first"}),
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let first: Value = serde_json::from_str(&common::text(first).await).unwrap();
    assert_eq!(
        call(
            &h,
            "author",
            "POST",
            &path,
            Some(Uuid::new_v4()),
            json!({"kind":"note","body":"second"})
        )
        .await
        .status(),
        StatusCode::CREATED
    );
    let page = call(
        &h,
        "author",
        "GET",
        &format!("{path}?kind=note&limit=1"),
        None,
        Value::Null,
    )
    .await;
    let page: Value = serde_json::from_str(&common::text(page).await).unwrap();
    let cursor = page["next_cursor"].as_str().unwrap();
    let next = call(
        &h,
        "author",
        "GET",
        &format!("{path}?kind=note&limit=1&cursor={cursor}"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(next.status(), StatusCode::OK);
    let next: Value = serde_json::from_str(&common::text(next).await).unwrap();
    assert_ne!(page["entries"][0]["id"], next["entries"][0]["id"]);
    assert_eq!(next["next_cursor"], Value::Null);
    assert_eq!(
        call(
            &h,
            "author",
            "GET",
            &format!("{path}?kind=comment&cursor={cursor}"),
            None,
            Value::Null
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let other = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Other".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let wrong_parent = format!(
        "/api/documents/{}/entries/{}",
        other.id,
        first["id"].as_str().unwrap()
    );
    assert_eq!(
        call(
            &h,
            "author",
            "PATCH",
            &wrong_parent,
            None,
            json!({"body":"changed"})
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let other_vault = h
        .state
        .vaults
        .provision_user(&UserId("author".into()))
        .await
        .unwrap();
    let hidden = h
        .app
        .clone()
        .oneshot(
            Request::get(format!("{path}?kind=note"))
                .header(header::COOKIE, "folio_session=author")
                .header("x-folio-vault", other_vault.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn personal_tokens_use_live_roles_and_document_write_scope_for_entries() {
    let (h, path) = setup().await;
    let token = call(&h, "author", "POST", "/api/auth/tokens", None, json!({"name":"entry API", "audience":"api", "scopes":["documents:read","documents:write"], "expires_in_days":1})).await;
    assert_eq!(token.status(), StatusCode::CREATED);
    let token: Value = serde_json::from_str(&common::text(token).await).unwrap();
    let bearer = token["token"].as_str().unwrap();
    let created = h
        .app
        .clone()
        .oneshot(
            Request::post(&path)
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .header("x-folio-vault", h.vault.to_string())
                .header("Idempotency-Key", Uuid::new_v4().to_string())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"kind":"note","body":"token entry"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let entry: Value = serde_json::from_str(&common::text(created).await).unwrap();
    assert_eq!(entry["created_by"], "author");
    let updated = h
        .app
        .clone()
        .oneshot(
            Request::patch(format!("{path}/{}", entry["id"].as_str().unwrap()))
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .header("x-folio-vault", h.vault.to_string())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"body":"updated through token"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    h.members
        .grant(
            h.vault,
            &Principal::User(UserId("author".into())),
            VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();
    let denied = h
        .app
        .clone()
        .oneshot(
            Request::patch(format!("{path}/{}", entry["id"].as_str().unwrap()))
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .header("x-folio-vault", h.vault.to_string())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"body":"forbidden"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn read_only_personal_token_cannot_create_entries() {
    let (h, path) = setup().await;
    let token = call(&h, "author", "POST", "/api/auth/tokens", None, json!({"name":"read entries", "audience":"api", "scopes":["documents:read"], "expires_in_days":1})).await;
    assert_eq!(token.status(), StatusCode::CREATED);
    let token: Value = serde_json::from_str(&common::text(token).await).unwrap();
    let bearer = token["token"].as_str().unwrap();
    let readable = h
        .app
        .clone()
        .oneshot(
            Request::get(format!("{path}?kind=note"))
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .header("x-folio-vault", h.vault.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(readable.status(), StatusCode::OK);
    let response = h
        .app
        .clone()
        .oneshot(
            Request::post(&path)
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .header("x-folio-vault", h.vault.to_string())
                .header("Idempotency-Key", Uuid::new_v4().to_string())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"kind":"note","body":"forbidden"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
