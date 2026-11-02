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
        .create(
            h.vault,
            folio_core::models::CreateDocument {
                title: "Waiting".into(),
                content: Some("read".into()),
                original_filename: None,
                mime_type: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let dt = h
        .state
        .document_type_repo
        .create(
            h.vault,
            folio_core::models::CreateDocumentType {
                name: "Invoice".into(),
            },
        )
        .await
        .unwrap();
    h.state
        .document_repo
        .create(
            h.vault,
            folio_core::models::CreateDocument {
                title: "Already filed".into(),
                content: Some("read".into()),
                original_filename: None,
                mime_type: None,
                document_type_id: Some(dt.id),
                ..Default::default()
            },
        )
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
async fn home_page_applies_the_tag_filter_together_with_other_filters() {
    let h = harness().await;
    let mk = |title: &str| folio_core::models::CreateDocument {
        title: title.into(),
        content: Some("read".into()),
        ..Default::default()
    };
    let tagged = h
        .state
        .document_repo
        .create(h.vault, mk("Tagged doc"))
        .await
        .unwrap();
    h.state
        .document_repo
        .create(h.vault, mk("Untagged doc"))
        .await
        .unwrap();
    let tag = h
        .state
        .tag_repo
        .create(
            h.vault,
            folio_core::models::CreateTag {
                name: "tax".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    h.state
        .document_repo
        .add_tag(h.vault, tagged.id, tag.id)
        .await
        .unwrap();

    let page = text(
        h.app
            .clone()
            .oneshot(get(&format!("/?tag_id={}", tag.id)))
            .await
            .unwrap(),
    )
    .await;
    assert!(page.contains("Tagged doc"));
    assert!(!page.contains("Untagged doc"));

    let corr = h
        .state
        .correspondent_repo
        .create(
            h.vault,
            folio_core::models::CreateCorrespondent {
                name: "ACME".into(),
            },
        )
        .await
        .unwrap();
    let combined = text(
        h.app
            .clone()
            .oneshot(get(&format!(
                "/?tag_id={}&correspondent_id={}",
                tag.id, corr.id
            )))
            .await
            .unwrap(),
    )
    .await;
    assert!(!combined.contains("Tagged doc"));
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

#[tokio::test]
async fn api_docs_page_and_its_viewer_script_are_self_contained() {
    let h = harness().await;

    let docs = h.app.clone().oneshot(get("/api/docs")).await.unwrap();
    assert_eq!(docs.status(), StatusCode::OK);
    assert!(text(docs).await.contains("/static/vendor/scalar.js"));

    let script = h
        .app
        .clone()
        .oneshot(get("/static/vendor/scalar.js"))
        .await
        .unwrap();
    assert_eq!(script.status(), StatusCode::OK);
}

#[tokio::test]
async fn oversized_upload_is_rejected_with_413_and_a_json_error() {
    let h = harness().await;
    let big = "x".repeat(51 * 1024 * 1024);
    let mut request = multipart(&[("file", Some("big.bin"), &big)]);
    *request.uri_mut() = "/api/documents/upload".parse().unwrap();

    let response = h.app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(text(response).await.contains(r#""error""#));
}

#[tokio::test]
async fn unreadable_files_say_so_instead_of_waiting_for_review() {
    use folio_core::models::{ProcessingStage, ProcessingStatus};

    let h = harness().await;
    let doc = h
        .state
        .document_repo
        .create(
            h.vault,
            folio_core::models::CreateDocument {
                title: "scan_0042.pdf".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.state
        .document_repo
        .set_stage_status(
            h.vault,
            doc.id,
            ProcessingStage::Ocr,
            ProcessingStatus::Failed,
        )
        .await
        .unwrap();

    let inbox = text(h.app.clone().oneshot(get("/")).await.unwrap()).await;
    assert!(inbox.contains("Couldn't read this file"));
    assert!(inbox.contains("1 file couldn't be read"));
    assert!(!inbox.contains("Needs a look"));
    assert!(!inbox.contains("needs a look"));

    let page = text(
        h.app
            .clone()
            .oneshot(get(&format!("/documents/{}", doc.id)))
            .await
            .unwrap(),
    )
    .await;
    assert!(page.contains("Folio couldn't read this file"));
    assert!(!page.contains("Reading"));
}

fn get_html(uri: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .uri(uri)
        .header(header::ACCEPT, "text/html,application/xhtml+xml")
        .body(axum::body::Body::empty())
        .unwrap()
}

#[tokio::test]
async fn missing_pages_answer_with_html_in_the_app_shell() {
    let h = harness().await;
    for uri in [
        "/documents/00000000-0000-0000-0000-000000000000",
        "/does-not-exist",
    ] {
        let response = h.app.clone().oneshot(get_html(uri)).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
        assert!(response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        let body = text(response).await;
        assert!(body.contains("<title>Not found - Folio</title>"), "{uri}");
        assert!(body.contains(r#"<html lang="en">"#));
        assert!(body.contains("Browse"), "{uri} keeps the sidebar");
        assert!(body.contains("Back to the inbox"));
    }
}

#[tokio::test]
async fn api_and_script_clients_keep_json_errors() {
    let h = harness().await;

    let api = h
        .app
        .clone()
        .oneshot(get_html(
            "/api/documents/00000000-0000-0000-0000-000000000000",
        ))
        .await
        .unwrap();
    assert_eq!(api.status(), StatusCode::NOT_FOUND);
    assert!(api.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .contains("json"));

    let script = h
        .app
        .clone()
        .oneshot(get("/documents/00000000-0000-0000-0000-000000000000"))
        .await
        .unwrap();
    assert_eq!(script.status(), StatusCode::NOT_FOUND);
    assert!(text(script).await.contains(r#""error""#));
}

fn form_post(uri: &str, body: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn settings_page_toggles_the_mcp_endpoint() {
    let h = harness().await;
    let page = text(h.app.clone().oneshot(get("/settings")).await.unwrap()).await;
    assert!(page.contains(r#"name="mcp_enabled""#));

    let saved = h
        .app
        .clone()
        .oneshot(form_post("/settings/mcp", "mcp_enabled=on"))
        .await
        .unwrap();
    assert!(saved.status().is_redirection());
    let stored = h.state.settings_repo.get("mcp.enabled").await.unwrap();
    assert_eq!(stored.unwrap().value, "true");
    let page = text(h.app.clone().oneshot(get("/settings")).await.unwrap()).await;
    assert!(page.contains(r#"name="mcp_enabled" checked"#));

    h.app
        .clone()
        .oneshot(form_post("/settings/mcp", ""))
        .await
        .unwrap();
    let stored = h.state.settings_repo.get("mcp.enabled").await.unwrap();
    assert_eq!(stored.unwrap().value, "false");
}
