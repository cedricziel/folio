mod common;

use axum::{body::Body, http::Request};
use folio_core::models::{CreateDocument, ProcessingStage, ProcessingStatus, ReviewOrigin};
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn review_put_requires_explicit_state_and_rejects_audit_spoofing() {
    let h = common::harness().await;
    let doc = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Review me".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let path = format!("/api/documents/{}/review", doc.id);
    for (body, status, reviewed) in [
        ("{}", 422, false),
        (r#"{"reviewed":null}"#, 422, false),
        (r#"{"reviewed":true,"reviewed_by":"forged"}"#, 422, false),
        (r#"{"reviewed":true}"#, 200, true),
        (r#"{"reviewed":true}"#, 200, true),
        (r#"{"reviewed":false}"#, 200, false),
    ] {
        let response = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(&path)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{body}");
        let saved = h
            .state
            .document_repo
            .find_by_id(h.vault, doc.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.reviewed_at.is_some(), reviewed);
        assert_eq!(saved.ocr_status, ProcessingStatus::Pending);
        assert_eq!(saved.created_at, doc.created_at);
        if reviewed {
            assert_eq!(saved.reviewed_by, None);
            assert_eq!(saved.review_origin, Some(ReviewOrigin::System));
        }
    }
    let response = h
        .app
        .clone()
        .oneshot(common::get(&format!("/api/documents/{}", doc.id)))
        .await
        .unwrap();
    let json: Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(json["reviewed"], false);
    assert_eq!(json["can_review_document"], true);
    h.state.document_repo.delete(h.vault, doc.id).await.unwrap();
    let response = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&path)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"reviewed":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn web_review_preserves_selected_vault_and_renders_the_opposite_action() {
    let h = common::harness().await;
    let doc = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Review me".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let detail = format!("/vaults/{}/documents/{}", h.vault, doc.id);
    for (body, label) in [
        ("reviewed=true", "Mark unreviewed"),
        ("reviewed=false", "Mark reviewed"),
    ] {
        let response = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("{detail}/review"))
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert_eq!(response.headers()["location"], detail);
        let response = h.app.clone().oneshot(common::get(&detail)).await.unwrap();
        let html = common::text(response).await;
        assert!(html.contains(label));
    }
}

#[tokio::test]
async fn invalid_document_date_bounds_are_rejected_by_list_and_browse() {
    let h = common::harness().await;
    for path in [
        "/api/documents?document_date_from=2026-11-03&document_date_to=2026-11-02",
        "/api/documents/browse?document_date_from=1900-02-29",
        "/api/documents?reviewed=maybe",
    ] {
        let response = h.app.clone().oneshot(common::get(path)).await.unwrap();
        assert_eq!(response.status(), 400, "{path}");
    }
    let response = h.app.clone().oneshot(common::get("/api/documents?reviewed=false&document_date_from=2000-02-29&document_date_to=2000-02-29")).await.unwrap();
    assert_eq!(response.status(), 200);
    let json: Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(json["supports_document_review_filters"], true);
}

#[tokio::test]
async fn inbox_review_filter_lists_reviewed_documents() {
    let h = common::harness().await;
    let doc = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Already reviewed".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.state
        .document_repo
        .complete_ocr(h.vault, doc.id, "text")
        .await
        .unwrap();
    h.state
        .document_repo
        .set_stage_status(
            h.vault,
            doc.id,
            ProcessingStage::Embed,
            ProcessingStatus::Done,
        )
        .await
        .unwrap();
    h.state
        .document_repo
        .set_reviewed(h.vault, doc.id, true)
        .await
        .unwrap();
    let response = h
        .app
        .clone()
        .oneshot(common::get("/?reviewed=true"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(common::text(response).await.contains("Already reviewed"));
}
