mod common;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::Response,
};
use folio_auth::{crypto::hash_token, Session, User};
use folio_core::models::{CreateDocument, Principal, UserId, VaultId, VaultRole};
use folio_infra::{Database, JobType};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn execute(h: &common::Harness, sql: String) {
    let Database::Sqlite(pool) = &h.database else {
        unreachable!()
    };
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(pool)
        .await
        .unwrap();
}

async fn member(h: &common::Harness, name: &str, vault: VaultId) {
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
        .grant(
            vault,
            &Principal::User(UserId(name.into())),
            VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();
}

async fn get(h: &common::Harness, path: &str, actor: Option<&str>, vault: VaultId) -> Response {
    let mut request = Request::get(path).header("x-folio-vault", vault.to_string());
    if let Some(actor) = actor {
        request = request.header(header::COOKIE, format!("folio_session={actor}"));
    }
    h.app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn detail_exposes_safe_persisted_stage_state_without_raw_queue_errors() {
    let h = common::harness().await;
    let doc = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Retrying".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let job = h
        .state
        .job_queue
        .enqueue(JobType::process_document(doc.id))
        .await
        .unwrap();
    execute(&h, format!("UPDATE jobs SET error = 'https://secret-provider/token?password=private' WHERE id = '{job}'; UPDATE processing_stage_runs SET state = 'waiting_to_retry', attempt = 2, queue_attempt = 2, reason_code = 'provider_failed', retry_at = '2026-11-03T00:00:10Z' WHERE document_id = '{}' AND stage = 'ocr';", doc.id)).await;
    let expected_run = h
        .state
        .document_repo
        .processing_stage_summaries(h.vault, doc.id)
        .await
        .unwrap()[0]
        .run_id;
    let response = get(&h, &format!("/api/documents/{}", doc.id), None, h.vault).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = common::text(response).await;
    let detail: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        detail["processing_stages"][0],
        json!({
            "run_id": expected_run, "revision_id": null, "stage": "ocr", "attempt": 2,
            "state": "waiting_to_retry", "reason_code": "provider_failed",
            "retry_at": "2026-11-03T00:00:10Z", "has_text": null,
        })
    );
    assert!(!body.contains("secret-provider"));
    assert!(!body.contains("password"));
    assert_eq!(detail["ocr_status"], "pending");
    let list = get(&h, "/api/documents", None, h.vault).await;
    let list: Value = serde_json::from_str(&common::text(list).await).unwrap();
    assert!(list["documents"][0].get("processing_stages").is_none());
}

#[tokio::test]
async fn legacy_detail_has_no_fabricated_stage_runs_or_revision() {
    let h = common::harness().await;
    let doc = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Legacy".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.state
        .document_repo
        .complete_ocr(h.vault, doc.id, "")
        .await
        .unwrap();
    let response = get(&h, &format!("/api/documents/{}", doc.id), None, h.vault).await;
    assert_eq!(response.status(), StatusCode::OK);
    let detail: Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(detail["processing_stages"], json!([]));
    assert_eq!(detail["ocr_status"], "done");
}

#[tokio::test]
async fn stage_detail_requires_vault_access_and_a_live_document() {
    let h = common::harness_requiring_login().await;
    let other = h.vault_repository.create("Other", None).await.unwrap().id;
    member(&h, "stage-viewer", h.vault).await;
    member(&h, "stage-outsider", other).await;
    let doc = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Private stage".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.state
        .job_queue
        .enqueue(JobType::process_document(doc.id))
        .await
        .unwrap();
    let path = format!("/api/documents/{}", doc.id);
    let response = get(&h, &path, Some("stage-viewer"), h.vault).await;
    assert_eq!(response.status(), StatusCode::OK);
    let detail: Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert!(!detail["processing_stages"].as_array().unwrap().is_empty());
    assert_eq!(detail["can_edit_document_date"], false);
    assert_eq!(
        get(&h, &path, None, h.vault).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&h, &path, Some("stage-outsider"), h.vault)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&h, &path, Some("stage-outsider"), other).await.status(),
        StatusCode::NOT_FOUND
    );
    h.state.document_repo.delete(h.vault, doc.id).await.unwrap();
    assert_eq!(
        get(&h, &path, Some("stage-viewer"), h.vault).await.status(),
        StatusCode::NOT_FOUND
    );
}
