use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use chrono::Duration as ChronoDuration;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::VerifyingKey;
use uuid::Uuid;

use super::*;

#[test]
fn provider_tokens_are_es256_with_key_and_team() {
    let key = parse_key(TEST_KEY).unwrap();
    let token = sign(&key, "KEY123", "TEAM456", 1_700_000_000);

    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);
    let header: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
    let claims: serde_json::Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    assert_eq!(header, json!({"alg": "ES256", "kid": "KEY123"}));
    assert_eq!(claims, json!({"iss": "TEAM456", "iat": 1_700_000_000}));
    let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
    VerifyingKey::from(&key)
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .unwrap();
}

#[test]
fn keys_must_be_pkcs8_p256() {
    assert!(parse_key("not a key").is_err());
    assert!(parse_key(TEST_KEY).is_ok());
}

#[test]
fn responses_map_to_outcomes() {
    let cases = [
        (200, "", PushOutcome::Delivered),
        (
            410,
            r#"{"reason":"Unregistered","timestamp":1700000000000}"#,
            PushOutcome::TokenInvalid {
                since: Utc.timestamp_millis_opt(1_700_000_000_000).single(),
            },
        ),
        (
            410,
            r#"{"reason":"ExpiredToken"}"#,
            PushOutcome::TokenInvalid { since: None },
        ),
        (
            400,
            r#"{"reason":"BadDeviceToken"}"#,
            PushOutcome::TokenInvalid { since: None },
        ),
        (
            400,
            r#"{"reason":"DeviceTokenNotForTopic"}"#,
            PushOutcome::TokenInvalid { since: None },
        ),
        (
            400,
            r#"{"reason":"BadCollapseId"}"#,
            PushOutcome::Rejected {
                reason: "BadCollapseId".into(),
            },
        ),
        (
            413,
            r#"{"reason":"PayloadTooLarge"}"#,
            PushOutcome::Rejected {
                reason: "PayloadTooLarge".into(),
            },
        ),
        (
            403,
            r#"{"reason":"InvalidProviderToken"}"#,
            PushOutcome::CredentialError {
                reason: "InvalidProviderToken".into(),
            },
        ),
        (
            429,
            r#"{"reason":"TooManyRequests"}"#,
            PushOutcome::Retry {
                reason: "TooManyRequests".into(),
            },
        ),
        (
            503,
            "",
            PushOutcome::Retry {
                reason: "HTTP 503".into(),
            },
        ),
    ];
    for (status, body, expected) in cases {
        assert_eq!(outcome(status, body), expected, "{status} {body}");
    }
}

#[derive(Default)]
struct Stub {
    requests: Mutex<Vec<(String, HeaderMap, serde_json::Value)>>,
    expire_first: Mutex<bool>,
}

async fn handle(
    State(stub): State<Arc<Stub>>,
    Path(token): Path<String>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, String) {
    stub.requests
        .lock()
        .unwrap()
        .push((token.clone(), headers, body));
    if std::mem::take(&mut *stub.expire_first.lock().unwrap()) {
        return (
            StatusCode::FORBIDDEN,
            r#"{"reason":"ExpiredProviderToken"}"#.into(),
        );
    }
    match token.as_str() {
        "dead" => (
            StatusCode::GONE,
            r#"{"reason":"Unregistered","timestamp":1700000000000}"#.into(),
        ),
        _ => (StatusCode::OK, String::new()),
    }
}

/// An HTTP/2 cleartext APNs stand-in on a free port
async fn stub() -> (Arc<Stub>, String) {
    let stub = Arc::new(Stub::default());
    let app = Router::new()
        .route("/3/device/{token}", post(handle))
        .with_state(stub.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (stub, base)
}

fn client(base: &str) -> ApnsClient {
    ApnsClient::new(ApnsCredentials {
        team_id: "TEAM456".into(),
        key_id: "KEY123".into(),
        topic: "com.cedricziel.folio".into(),
        private_key: TEST_KEY.into(),
        base_url: Some(base.into()),
    })
    .unwrap()
}

fn message(token: &str, push_type: PushType) -> PushMessage {
    PushMessage {
        apns_id: Uuid::new_v4(),
        token: token.into(),
        environment: PushEnvironment::Production,
        push_type,
        priority: 10,
        collapse_id: Some("batch".into()),
        expiration: Utc::now() + ChronoDuration::hours(24),
        payload: json!({"aps": {"alert": {"title": "Folio", "body": "1 document needs review"}}}),
    }
}

#[tokio::test]
async fn sends_alerts_over_http2_with_apns_headers() {
    let (stub, base) = stub().await;
    let client = client(&base);
    let message = message("abcd", PushType::Alert);

    assert_eq!(client.send(&message).await, PushOutcome::Delivered);

    let requests = stub.requests.lock().unwrap();
    let (token, headers, body) = &requests[0];
    assert_eq!(token, "abcd");
    let header = |name: &str| headers.get(name).unwrap().to_str().unwrap().to_string();
    assert_eq!(header("apns-push-type"), "alert");
    assert_eq!(header("apns-topic"), "com.cedricziel.folio");
    assert_eq!(header("apns-priority"), "10");
    assert_eq!(header("apns-collapse-id"), "batch");
    assert_eq!(header("apns-id"), message.apns_id.to_string());
    assert_eq!(
        header("apns-expiration"),
        message.expiration.timestamp().to_string()
    );
    assert!(header("authorization")
        .to_ascii_lowercase()
        .starts_with("bearer ey"));
    assert_eq!(*body, message.payload);
}

#[tokio::test]
async fn widget_reloads_use_their_own_topic() {
    let (stub, base) = stub().await;
    let mut message = message("abcd", PushType::Widgets);
    message.collapse_id = None;

    client(&base).send(&message).await;

    let requests = stub.requests.lock().unwrap();
    let headers = &requests[0].1;
    assert_eq!(headers["apns-push-type"], "widgets");
    assert_eq!(
        headers["apns-topic"],
        "com.cedricziel.folio.push-type.widgets"
    );
    assert!(headers.get("apns-collapse-id").is_none());
}

#[tokio::test]
async fn a_dead_token_reports_when_it_died() {
    let (_, base) = stub().await;

    let outcome = client(&base).send(&message("dead", PushType::Alert)).await;

    assert_eq!(
        outcome,
        PushOutcome::TokenInvalid {
            since: Utc.timestamp_millis_opt(1_700_000_000_000).single()
        }
    );
}

#[tokio::test]
async fn an_expired_provider_token_is_signed_again_once() {
    let (stub, base) = stub().await;
    *stub.expire_first.lock().unwrap() = true;
    let client = client(&base);

    assert_eq!(
        client.send(&message("abcd", PushType::Alert)).await,
        PushOutcome::Delivered
    );
    assert_eq!(stub.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn an_unreachable_server_is_retried_later() {
    let client = client("http://127.0.0.1:9");
    assert!(matches!(
        client.send(&message("abcd", PushType::Alert)).await,
        PushOutcome::Retry { .. }
    ));
}
