mod common;

use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::Response,
};
use folio_auth::{crypto::hash_token, Session, User};
use folio_core::{
    models::{CreateDocument, Principal, UserId, VaultId, VaultRole},
    services::WorkflowEngine,
};
use folio_infra::workflow::{RunOutcome, StorageFileMetadata, WorkflowRunner};
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
async fn entry_workflow_keeps_running_without_granting_access_to_its_revoked_creator() {
    let (h, entry_path) = setup().await;
    let creator = "workflow-creator";
    let owner = "remaining-owner";
    member(&h, creator, h.vault, VaultRole::Owner).await;
    member(&h, owner, h.vault, VaultRole::Owner).await;

    let tag = call(
        &h,
        owner,
        "POST",
        "/api/tags",
        None,
        json!({"name": "Discussed"}),
    )
    .await;
    assert_eq!(tag.status(), StatusCode::OK);
    let tag: Value = serde_json::from_str(&common::text(tag).await).unwrap();
    let tag_id: Uuid = serde_json::from_value(tag["id"].clone()).unwrap();
    let workflow = call(
        &h,
        creator,
        "POST",
        "/api/workflows",
        None,
        json!({
            "name": "Tag discussed documents",
            "trigger": "document_note.created",
            "actions": [{"type": "add_tags", "tag_ids": [tag_id]}]
        }),
    )
    .await;
    let status = workflow.status();
    let workflow = common::text(workflow).await;
    assert_eq!(status, StatusCode::CREATED, "{workflow}");
    let workflow: Value = serde_json::from_str(&workflow).unwrap();
    let workflow_id: Uuid = serde_json::from_value(workflow["id"].clone()).unwrap();
    assert_eq!(workflow["created_by"], creator);
    let workflow_path = format!("/api/workflows/{workflow_id}");
    let runs_path = format!("{workflow_path}/runs");
    for path in [&workflow_path, &runs_path] {
        assert_eq!(
            call(&h, creator, "GET", path, None, Value::Null)
                .await
                .status(),
            StatusCode::OK
        );
    }

    h.members
        .revoke(h.vault, &Principal::User(UserId(creator.into())))
        .await
        .unwrap();
    let entry = call(
        &h,
        "author",
        "POST",
        &entry_path,
        Some(Uuid::new_v4()),
        json!({"kind": "note", "body": "This note triggers a vault workflow."}),
    )
    .await;
    assert_eq!(entry.status(), StatusCode::CREATED);
    let entry: Value = serde_json::from_str(&common::text(entry).await).unwrap();
    let entry_id: Uuid = serde_json::from_value(entry["id"].clone()).unwrap();
    let document_id: Uuid = serde_json::from_value(entry["document_id"].clone()).unwrap();
    let event = h
        .state
        .outbox
        .replay(0, chrono::Utc::now() - chrono::Duration::hours(1), 100)
        .await
        .unwrap()
        .into_iter()
        .find(|event| {
            event.envelope.as_ref().is_some_and(|envelope| {
                envelope.event_type() == "document_note.created"
                    && envelope.subject().id == entry_id
            })
        })
        .expect("the committed note creation event");

    let engine = WorkflowEngine::new(
        h.state.workflow_repo.clone(),
        h.state.document_repo.clone(),
        h.state.correspondent_repo.clone(),
        h.state.document_type_repo.clone(),
        Arc::new(StorageFileMetadata::new(h.state.store.clone())),
        h.state.public_url.clone(),
    );
    let runs = engine.handle(&event).await.unwrap();
    let [run] = runs.as_slice() else {
        panic!("expected one workflow run, found {}", runs.len());
    };
    assert_eq!(run.workflow_id, workflow_id);
    let runner = WorkflowRunner::new(
        h.state.workflow_repo.clone(),
        h.database.repositories().await.workflow_documents,
        h.state.webhook_client.clone(),
    );
    assert_eq!(runner.execute(run.id).await.unwrap(), RunOutcome::Succeeded);
    let tags = h
        .state
        .document_repo
        .get_tags(h.vault, document_id)
        .await
        .unwrap();
    assert!(tags.iter().any(|tag| tag.id == tag_id));

    for path in [&workflow_path, &runs_path] {
        assert_eq!(
            call(&h, creator, "GET", path, None, Value::Null)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    let accessible_runs = call(&h, owner, "GET", &runs_path, None, Value::Null).await;
    assert_eq!(accessible_runs.status(), StatusCode::OK);
    let accessible_runs: Value =
        serde_json::from_str(&common::text(accessible_runs).await).unwrap();
    assert_eq!(accessible_runs[0]["id"], run.id.to_string());
    assert_eq!(accessible_runs[0]["status"], "succeeded");
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
