//! Route-level tests for the inbox, document pages and uploads.

mod common;

use axum::http::{header, StatusCode};
use common::{get, harness, multipart, text};
use tower::ServiceExt;

#[tokio::test]
async fn upload_answers_with_streams_that_add_the_row() {
    let h = harness().await;
    let mut request = multipart(&[
        ("title", None, "Invoice"),
        ("file", Some("invoice.png"), "bytes"),
    ]);
    *request.uri_mut() = "/documents/upload".parse().unwrap();

    let response = h.app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/vnd.turbo-stream.html"
    );
    let body = text(response).await;
    assert!(body.contains(r#"action="prepend" target="documents-rows""#));
    assert!(body.contains("Invoice"));
    assert!(body.contains("Reading"));
    assert!(body.contains(r#"target="inbox-count""#));
}

#[tokio::test]
async fn inbox_lists_unfiled_documents_and_all_documents_lists_everything() {
    let h = harness().await;
    let waiting = h
        .state
        .document_repo
        .create(folio_core::models::CreateDocument {
            title: "Waiting".into(),
            content: Some("read".into()),
            original_filename: None,
            mime_type: None,
            correspondent_id: None,
            document_type_id: None,
        })
        .await
        .unwrap();
    let dt = h
        .state
        .document_type_repo
        .create(folio_core::models::CreateDocumentType {
            name: "Invoice".into(),
        })
        .await
        .unwrap();
    h.state
        .document_repo
        .create(folio_core::models::CreateDocument {
            title: "Already filed".into(),
            content: Some("read".into()),
            original_filename: None,
            mime_type: None,
            correspondent_id: None,
            document_type_id: Some(dt.id),
        })
        .await
        .unwrap();

    let inbox = text(h.app.clone().oneshot(get("/")).await.unwrap()).await;
    assert!(inbox.contains("Waiting"));
    assert!(!inbox.contains("Already filed"));
    assert!(inbox.contains("1 document needs a look"));
    assert!(inbox.contains(&format!("/documents/{}", waiting.id)));

    let all = text(h.app.clone().oneshot(get("/?view=all")).await.unwrap()).await;
    assert!(all.contains("Waiting"));
    assert!(all.contains("Already filed"));
}

#[tokio::test]
async fn every_page_renders() {
    let h = harness().await;
    for uri in ["/", "/?view=all", "/manage", "/settings"] {
        let response = h.app.clone().oneshot(get(uri)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let body = text(response).await;
        assert!(body.contains("Browse"), "{uri} has the sidebar");
        assert!(!body.contains("Error rendering"), "{uri}");
    }
}
