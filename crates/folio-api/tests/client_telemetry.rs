mod common;

use axum::body::{to_bytes, Body};
use axum::http::Request;
use tower::ServiceExt;

#[tokio::test]
async fn client_configuration_is_disabled_without_an_exporter() {
    let h = common::harness().await;
    let response = h
        .app
        .oneshot(
            Request::builder()
                .uri("/api/telemetry/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["enabled"], false);
    assert_eq!(body["serviceNamespace"], "folio");
    assert!(body.get("headers").is_none());
}

#[tokio::test]
async fn client_configuration_requires_authentication() {
    let h = common::harness_requiring_login().await;
    let response = h
        .app
        .oneshot(
            Request::builder()
                .uri("/api/telemetry/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

use axum::http::{header, HeaderMap, StatusCode};
use axum::{extract::State, response::IntoResponse, routing::any, Router};
use folio_api::{client_telemetry::ClientTelemetry, create_router, AuthState};
use std::sync::{Arc, Mutex};

struct Captured {
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}
struct Receiver {
    url: String,
    requests: Arc<Mutex<Vec<Captured>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Receiver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn receive(
    State((requests, status)): State<(Arc<Mutex<Vec<Captured>>>, StatusCode)>,
    request: Request<Body>,
) -> impl IntoResponse {
    let path = request.uri().path().to_owned();
    let headers = request.headers().clone();
    let body = to_bytes(request.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    requests.lock().unwrap().push(Captured {
        path,
        headers,
        body,
    });
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::LOCATION, "/redirected"),
        ],
        "{}",
    )
}
impl Receiver {
    async fn start(status: StatusCode) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let router = Router::new()
            .fallback(any(receive))
            .with_state((requests.clone(), status));
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            url,
            requests,
            task,
        }
    }
    async fn app(&self) -> (Router, common::Harness) {
        let mut h = common::harness().await;
        h.state.client_telemetry = ClientTelemetry::from_lookup(|key| match key {
            "FOLIO_TELEMETRY__CLIENTS_ENABLED" => Some("true".into()),
            "FOLIO_TELEMETRY__OTLP_HTTP_ENDPOINT" => Some(self.url.clone()),
            "OTEL_EXPORTER_OTLP_HEADERS" => Some(
                "authorization=Bearer%20upstream-only,x-tenant-id=homelab,x-dataset-id=apps".into(),
            ),
            _ => None,
        })
        .unwrap()
        .map(Arc::new);
        let app = create_router(
            h.state.clone(),
            AuthState::new(h.auth_store.clone(), None).with_required(false),
        );
        (app, h)
    }
}

#[tokio::test]
async fn relay_preserves_otlp_body_and_uses_only_server_credentials() {
    let receiver = Receiver::start(StatusCode::OK).await;
    let (app, harness) = receiver.app().await;
    harness
        .auth_store
        .create_user(folio_auth::User {
            id: "telemetry-user".into(),
            username: "telemetry-user".into(),
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
    harness
        .auth_store
        .insert_client(folio_auth::OAuthClient {
            client_id: "telemetry-client".into(),
            client_name: "Telemetry fixture".into(),
            redirect_uris: vec!["http://localhost/callback".into()],
            created_at: 1,
        })
        .await
        .unwrap();
    harness
        .auth_store
        .insert_access_token(folio_auth::AccessToken {
            token_hash: folio_auth::crypto::hash_token("client-token"),
            client_id: "telemetry-client".into(),
            user_id: "telemetry-user".into(),
            scopes: vec!["documents:read".into()],
            audience: "http://localhost/api".into(),
            expires_at: i64::MAX,
            grant_id: "telemetry-grant".into(),
        })
        .await
        .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/telemetry/v1/traces")
                .header(header::CONTENT_TYPE, "application/x-protobuf")
                .header(header::AUTHORIZATION, "Bearer client-token")
                .header(header::COOKIE, "private-session=not-for-upstream")
                .header("x-tenant-id", "attacker")
                .header(
                    "traceparent",
                    "00-11111111111111111111111111111111-1111111111111111-01",
                )
                .body(Body::from(vec![10_u8, 0]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let logs = br#"{"resourceLogs":[{"scopeLogs":[{"logRecords":[{"eventName":"folio.action","attributes":[{"key":"session.id","value":{"stringValue":"0b7c2a52-4f0e-4c1e-9a43-2f6a0d1c9e11"}},{"key":"app.screen.name","value":{"stringValue":"inbox"}},{"key":"folio.action.name","value":{"stringValue":"document.delete"}}]}]}]}]}"#;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/telemetry/v1/logs")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, "Bearer client-token")
                .body(Body::from(logs.as_slice()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let config = app
        .oneshot(
            Request::builder()
                .uri("/api/telemetry/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let config = to_bytes(config.into_body(), 4096).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&config).unwrap();
    assert_eq!(value["enabled"], true);
    assert_eq!(value["endpoint"], "/api/telemetry");
    assert!(!String::from_utf8_lossy(&config).contains("upstream-only"));
    assert!(!String::from_utf8_lossy(&config).contains(&receiver.url));
    let requests = receiver.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/v1/logs");
    assert_eq!(
        requests[1].body, logs,
        "client attributes reach the collector unchanged"
    );
    assert_eq!(requests[0].path, "/v1/traces");
    assert_eq!(requests[0].body, [10, 0]);
    assert_eq!(
        requests[0].headers[header::AUTHORIZATION],
        "Bearer upstream-only"
    );
    assert_eq!(requests[0].headers["x-tenant-id"], "homelab");
    assert_eq!(requests[0].headers["x-dataset-id"], "apps");
    assert!(!requests[0].headers.contains_key(header::COOKIE));
    assert!(!requests[0].headers.contains_key("traceparent"));
}

#[tokio::test]
async fn oversized_or_encoded_payloads_do_not_reach_the_collector() {
    let receiver = Receiver::start(StatusCode::OK).await;
    let (app, _harness) = receiver.app().await;
    let oversized = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/telemetry/v1/logs")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(vec![b' '; 1024 * 1024 + 1]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), 413);
    let encoded = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/telemetry/v1/metrics")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_ENCODING, "gzip")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(encoded.status(), 415);
    assert!(receiver.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn collector_redirects_are_not_followed_with_export_credentials() {
    let receiver = Receiver::start(StatusCode::TEMPORARY_REDIRECT).await;
    let (app, _harness) = receiver.app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/telemetry/v1/traces")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 502);
    assert_eq!(receiver.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn an_incomplete_payload_times_out_without_occupying_a_collector_slot() {
    let receiver = Receiver::start(StatusCode::OK).await;
    let (app, _harness) = receiver.app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/telemetry/v1/traces")
                .header(header::CONTENT_TYPE, "application/x-protobuf")
                .body(Body::from_stream(futures::stream::pending::<
                    Result<axum::body::Bytes, std::io::Error>,
                >()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 408);
    assert!(receiver.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn export_budget_rejects_excess_requests_without_hiding_configuration() {
    let receiver = Receiver::start(StatusCode::OK).await;
    let (app, _harness) = receiver.app().await;
    for _ in 0..120 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/telemetry/v1/metrics")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
    let excess = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/telemetry/v1/metrics")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(excess.status(), 429);
    assert_eq!(excess.headers()[header::RETRY_AFTER], "60");
    assert_eq!(receiver.requests.lock().unwrap().len(), 120);
    let config = app
        .oneshot(
            Request::builder()
                .uri("/api/telemetry/config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(config.status(), 200);
}
