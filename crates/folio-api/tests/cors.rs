//! Cross-origin browser requests must get no CORS allowance.

mod common;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use common::{harness, harness_requiring_login, Harness};
use tower::ServiceExt;

const EVIL: &str = "https://evil.example";

fn preflight() -> Request<Body> {
    Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/documents")
        .header(header::ORIGIN, EVIL)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "DELETE")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
        .body(Body::empty())
        .unwrap()
}

fn cross_origin_get() -> Request<Body> {
    Request::builder()
        .uri("/api/documents")
        .header(header::ORIGIN, EVIL)
        .body(Body::empty())
        .unwrap()
}

fn assert_no_cors(response: &axum::response::Response) {
    let cors: Vec<_> = response
        .headers()
        .keys()
        .filter(|name| name.as_str().starts_with("access-control-"))
        .collect();
    assert!(cors.is_empty(), "unexpected CORS headers: {cors:?}");
}

async fn assert_denied_cross_origin(h: &Harness) {
    let response = h.app.clone().oneshot(preflight()).await.unwrap();
    assert_no_cors(&response);
    let response = h.app.clone().oneshot(cross_origin_get()).await.unwrap();
    assert_no_cors(&response);
}

#[tokio::test]
async fn cross_origin_gets_no_cors_headers_when_auth_is_off() {
    assert_denied_cross_origin(&harness().await).await;
}

#[tokio::test]
async fn cross_origin_gets_no_cors_headers_when_auth_is_required() {
    assert_denied_cross_origin(&harness_requiring_login().await).await;
}

#[tokio::test]
async fn same_origin_request_still_succeeds() {
    let h = harness().await;
    let request = Request::builder()
        .uri("/api/documents")
        .header(header::HOST, "localhost")
        .header("sec-fetch-site", "same-origin")
        .body(Body::empty())
        .unwrap();
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_no_cors(&response);
}

fn settings_post(headers: &[(&str, &str)]) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/settings/mcp")
        .header(header::HOST, "192.168.1.5:3000")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder.body(Body::from("mcp_enabled=on")).unwrap()
}

async fn post_changes_setting(headers: &[(&str, &str)]) -> bool {
    changes_setting(&harness().await, settings_post(headers)).await
}

async fn changes_setting(h: &Harness, request: Request<Body>) -> bool {
    let response = h.app.clone().oneshot(request).await.unwrap();
    let stored = h.state.settings_repo.get("mcp.enabled").await.unwrap();
    let changed = stored.is_some_and(|s| s.value == "true");
    if !changed {
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    changed
}

#[tokio::test]
async fn cross_site_form_post_is_rejected_when_auth_is_off() {
    assert!(!post_changes_setting(&[("sec-fetch-site", "cross-site")]).await);
}

#[tokio::test]
async fn post_from_another_origin_is_rejected_without_fetch_metadata() {
    assert!(!post_changes_setting(&[(header::ORIGIN.as_str(), EVIL)]).await);
}

#[tokio::test]
async fn same_origin_and_header_less_posts_still_work_when_auth_is_off() {
    assert!(post_changes_setting(&[("sec-fetch-site", "same-origin")]).await);
    assert!(post_changes_setting(&[(header::ORIGIN.as_str(), "http://192.168.1.5:3000")]).await);
    assert!(post_changes_setting(&[]).await);
}

#[tokio::test]
async fn opaque_and_same_site_posts_are_rejected_when_auth_is_off() {
    assert!(!post_changes_setting(&[(header::ORIGIN.as_str(), "null")]).await);
    assert!(!post_changes_setting(&[("sec-fetch-site", "same-site")]).await);
}

#[tokio::test]
async fn forwarded_host_proxy_posts_are_accepted_when_auth_is_off() {
    let headers = [
        (header::ORIGIN.as_str(), "http://folio.example"),
        ("x-forwarded-host", "folio.example"),
    ];
    assert!(post_changes_setting(&headers).await);
}

async fn with_public_url() -> Harness {
    common::harness_with(Vec::new(), |store| {
        folio_api::AuthState::new(store, Some("http://folio.example:8443".into()))
            .with_required(false)
    })
    .await
}

#[tokio::test]
async fn configured_public_url_origin_is_accepted_when_auth_is_off() {
    let own = settings_post(&[(header::ORIGIN.as_str(), "http://folio.example:8443")]);
    assert!(changes_setting(&with_public_url().await, own).await);
    let other = settings_post(&[(header::ORIGIN.as_str(), EVIL)]);
    assert!(!changes_setting(&with_public_url().await, other).await);
}

fn upload(headers: &[(&str, &str)], uri: &str) -> Request<Body> {
    let mut request = common::multipart(&[("title", None, "Doc"), ("file", Some("a.png"), "x")]);
    *request.uri_mut() = uri.parse().unwrap();
    for (name, value) in headers {
        request.headers_mut().insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    request
}

#[tokio::test]
async fn http2_post_without_host_header_survives_the_vault_path_rewrite() {
    let h = harness().await;
    let uri = format!(
        "http://192.168.1.5:3000/vaults/{}/documents/upload",
        h.vault
    );
    let request = upload(&[("origin", "http://192.168.1.5:3000")], &uri);
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert_ne!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn cross_site_multipart_upload_is_rejected_when_auth_is_off() {
    let h = harness().await;
    let request = upload(&[("sec-fetch-site", "cross-site")], "/documents/upload");
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
