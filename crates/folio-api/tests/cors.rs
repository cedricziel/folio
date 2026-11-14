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
    let h = harness().await;
    let response = h.app.clone().oneshot(settings_post(headers)).await.unwrap();
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
