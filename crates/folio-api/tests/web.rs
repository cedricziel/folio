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
            ..Default::default()
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
            document_type_id: Some(dt.id),
            ..Default::default()
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
    for uri in [
        "/",
        "/?view=all",
        "/manage",
        "/settings",
        "/settings/sources",
    ] {
        let response = h.app.clone().oneshot(get(uri)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let body = text(response).await;
        assert!(body.contains("Browse"), "{uri} has the sidebar");
        assert!(!body.contains("Error rendering"), "{uri}");
    }
}

#[tokio::test]
async fn vendored_scripts_and_fonts_are_served_without_a_cdn() {
    let h = harness().await;

    let turbo = h
        .app
        .clone()
        .oneshot(get("/static/vendor/turbo.js"))
        .await
        .unwrap();
    assert_eq!(turbo.status(), StatusCode::OK);
    assert!(turbo.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .contains("javascript"));
    assert!(turbo.headers()[header::CACHE_CONTROL]
        .to_str()
        .unwrap()
        .contains("max-age=604800"));
    let etag = turbo.headers()[header::ETAG].clone();

    let again = h
        .app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/static/vendor/turbo.js")
                .header(header::IF_NONE_MATCH, etag)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::NOT_MODIFIED);

    let font = h
        .app
        .clone()
        .oneshot(get("/static/fonts/geist-latin-wght-normal.woff2"))
        .await
        .unwrap();
    assert_eq!(font.status(), StatusCode::OK);
    assert_eq!(font.headers()[header::CONTENT_TYPE], "font/woff2");

    let missing = h.app.clone().oneshot(get("/static/nope.js")).await.unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let page = text(h.app.clone().oneshot(get("/")).await.unwrap()).await;
    assert!(!page.contains("esm.sh"));
    assert!(!page.contains("googleapis"));
}
