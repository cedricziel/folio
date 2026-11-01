//! Installable app routes: manifest, service worker and share target.

mod common;

use axum::http::{header, StatusCode};
use common::{get, harness, multipart, text};
use tower::ServiceExt;

#[tokio::test]
async fn share_target_stores_the_file_and_returns_to_the_inbox() {
    let h = harness().await;
    let mut request = multipart(&[("file", Some("scan.png"), "not really a png")]);
    *request.uri_mut() = "/share".parse().unwrap();

    let response = h.app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/");
    let docs = h.state.document_repo.list(None).await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].original_filename.as_deref(), Some("scan.png"));
}

#[tokio::test]
async fn share_target_without_a_file_is_a_bad_request() {
    let h = harness().await;
    let mut request = multipart(&[("title", None, "hello")]);
    *request.uri_mut() = "/share".parse().unwrap();

    let response = h.app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn manifest_declares_the_share_target() {
    let h = harness().await;

    let response = h
        .app
        .clone()
        .oneshot(get("/manifest.webmanifest"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/manifest+json"
    );
    let manifest: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(manifest["name"], "Folio");
    assert_eq!(manifest["share_target"]["action"], "/share");
    assert_eq!(manifest["share_target"]["method"], "POST");
}

#[tokio::test]
async fn service_worker_is_served_from_the_root_scope() {
    let h = harness().await;

    let response = h.app.clone().oneshot(get("/sw.js")).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["service-worker-allowed"], "/");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
    assert!(text(response).await.contains("addEventListener(\"fetch\""));
}
