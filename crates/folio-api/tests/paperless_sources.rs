//! Paperless sources through the real router: settings, token handling and access.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use folio_core::models::SourceConfig;
use serde_json::{json, Value};
use tower::ServiceExt;

use common::Harness;

const TOKEN: &str = "paperless-secret-token";

async fn send(
    app: &Harness,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    let body = match body {
        Some(json) => {
            builder = builder.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let text = common::text(response).await;
    let value = if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    };
    (status, value)
}

fn paperless_body(url: &str, token: &str) -> Value {
    json!({
        "name": "Paperless",
        "config": {"type": "paperless", "url": url, "token": token, "sync_mode": "mirror"}
    })
}

async fn stored_config(app: &Harness, id: &str) -> folio_core::models::PaperlessConfig {
    let def = app
        .state
        .ingestion
        .managed
        .find(app.vault, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let SourceConfig::Paperless(config) = def.config else {
        panic!("paperless")
    };
    config
}

#[tokio::test]
async fn tokens_are_redacted_and_a_placeholder_keeps_the_stored_one_for_the_same_url() {
    let app = common::harness().await;

    let (status, created) = send(
        &app,
        "POST",
        "/api/sources",
        None,
        Some(paperless_body("https://paperless.example", TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["config"]["token"], "********");
    assert_eq!(created["config"]["sync_mode"], "mirror");
    assert_eq!(created["config"]["poll_interval_secs"], 900);
    let id = created["id"].as_str().unwrap().to_string();

    for uri in ["/api/sources".to_string(), format!("/api/sources/{id}")] {
        let (_, listed) = send(&app, "GET", &uri, None, None).await;
        assert!(!listed.to_string().contains(TOKEN), "{uri}: {listed}");
    }

    let (status, updated) = send(
        &app,
        "PUT",
        &format!("/api/sources/{id}"),
        None,
        Some(json!({
            "name": "Paperless",
            "config": {"type": "paperless", "url": "https://paperless.example", "token": "********", "sync_mode": "new_only"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["config"]["token"], "********");
    let stored = stored_config(&app, &id).await;
    assert_eq!(stored.token.as_deref(), Some(TOKEN));
    assert_eq!(
        stored.sync_mode,
        folio_core::models::PaperlessSyncMode::NewOnly
    );
}

#[tokio::test]
async fn a_new_url_with_the_placeholder_asks_for_the_token_and_changes_nothing() {
    let app = common::harness().await;
    let (_, created) = send(
        &app,
        "POST",
        "/api/sources",
        None,
        Some(paperless_body("https://paperless.example", TOKEN)),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();

    for token in [json!("********"), json!(""), Value::Null] {
        let mut body = paperless_body("https://elsewhere.example", "");
        body["config"]["token"] = token.clone();
        let (status, error) =
            send(&app, "PUT", &format!("/api/sources/{id}"), None, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{token}: {error}");
        assert!(
            error["error"].as_str().unwrap().contains("token"),
            "{error}"
        );
    }
    let stored = stored_config(&app, &id).await;
    assert_eq!(stored.url, "https://paperless.example");
    assert_eq!(stored.token.as_deref(), Some(TOKEN));

    let (status, _) = send(
        &app,
        "PUT",
        &format!("/api/sources/{id}"),
        None,
        Some(paperless_body("https://elsewhere.example", "new-token")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let stored = stored_config(&app, &id).await;
    assert_eq!(stored.url, "https://elsewhere.example");
    assert_eq!(stored.token.as_deref(), Some("new-token"));
}

#[tokio::test]
async fn invalid_settings_are_rejected_naming_the_field() {
    let app = common::harness().await;
    let cases = [
        (
            "url",
            json!({"url": "ftp://paperless.example", "token": TOKEN}),
        ),
        ("url", json!({"url": "not a url", "token": TOKEN})),
        (
            "poll_interval_secs",
            json!({"url": "https://p.example", "token": TOKEN, "poll_interval_secs": 30}),
        ),
        (
            "token_file",
            json!({"url": "https://p.example", "token_file": "/etc/passwd"}),
        ),
        (
            "token_file",
            json!({"url": "https://p.example", "token": TOKEN, "token_file": "/etc/passwd"}),
        ),
        ("token", json!({"url": "https://p.example"})),
        (
            "default_currency",
            json!({"url": "https://p.example", "token": TOKEN, "default_currency": "euro"}),
        ),
    ];
    for (field, mut config) in cases {
        config["type"] = json!("paperless");
        let body = json!({"name": "Paperless", "config": config});
        let (status, error) = send(&app, "POST", "/api/sources", None, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {error}");
        assert!(
            error["error"].as_str().unwrap().contains(field),
            "{field}: {error}"
        );
    }
    let (_, listed) = send(&app, "GET", "/api/sources", None, None).await;
    assert!(listed.as_array().unwrap().is_empty(), "{listed}");

    let (_, created) = send(
        &app,
        "POST",
        "/api/sources",
        None,
        Some(paperless_body("https://paperless.example", TOKEN)),
    )
    .await;
    let id = created["id"].as_str().unwrap();
    let mut body = paperless_body("https://paperless.example", "********");
    body["config"]["token_file"] = json!("/etc/passwd");
    let (status, error) = send(&app, "PUT", &format!("/api/sources/{id}"), None, Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error["error"].as_str().unwrap().contains("token_file"));
}
