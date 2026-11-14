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
