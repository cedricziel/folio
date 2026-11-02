//! Ingestion source API and pages, driven through the real router.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use folio_core::models::{FolderConfig, IngestionSourceDef, SourceConfig, SourceOrigin};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use common::{harness_with_sources, Harness};

async fn test_app(configured: Vec<IngestionSourceDef>) -> Harness {
    harness_with_sources(configured).await
}

async fn send(app: &Harness, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
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
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn json_request(
    app: &Harness,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, text) = send(app, method, uri, body).await;
    let value = if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    };
    (status, value)
}

fn configured_folder(path: &str) -> IngestionSourceDef {
    IngestionSourceDef {
        vault_id: None,
        id: Uuid::new_v4(),
        name: "scans".into(),
        enabled: true,
        origin: SourceOrigin::Config,
        config: SourceConfig::Folder(
            serde_json::from_value::<FolderConfig>(json!({ "path": path })).unwrap(),
        ),
    }
}

fn mail_body(password: &str) -> Value {
    json!({
        "name": "Work mail",
        "config": {
            "type": "email",
            "host": "imap.example.test",
            "username": "me",
            "password": password,
        }
    })
}

#[tokio::test]
async fn passwords_are_masked_and_a_masked_update_keeps_the_stored_one() {
    let app = test_app(Vec::new()).await;

    let (status, created) =
        json_request(&app, "POST", "/api/sources", Some(mail_body("hunter2"))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["vault_id"], app.vault.to_string());
    assert_eq!(created["config"]["password"], "********");
    assert_eq!(created["config"]["port"], 993);
    let id = created["id"].as_str().unwrap().to_string();

    let (_, listed) = json_request(&app, "GET", "/api/sources", None).await;
    assert!(!listed.to_string().contains("hunter2"));

    let (status, _) = json_request(
        &app,
        "PUT",
        &format!("/api/sources/{id}"),
        Some(mail_body("********")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let stored = app
        .state
        .ingestion
        .managed
        .find(app.vault, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let SourceConfig::Email(email) = stored.config else {
        panic!("email")
    };
    assert_eq!(email.password.as_deref(), Some("hunter2"));

    let (status, _) = json_request(&app, "DELETE", &format!("/api/sources/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn password_files_are_refused_for_managed_sources() {
    let app = test_app(Vec::new()).await;
    let mut body = mail_body("x");
    body["config"]["password_file"] = json!("/etc/passwd");
    let (status, _) = json_request(&app, "POST", "/api/sources", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn configured_sources_are_listed_read_only_and_can_be_tested() {
    let missing = configured_folder("/definitely/not/here");
    let existing = std::env::temp_dir().join(format!("folio-sources-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&existing).unwrap();
    let mut ok = configured_folder(existing.to_str().unwrap());
    ok.name = "tmp".into();
    let (missing_id, ok_id) = (missing.id, ok.id);
    let app = test_app(vec![missing, ok]).await;

    let (_, listed) = json_request(&app, "GET", "/api/sources", None).await;
    assert_eq!(listed[0]["origin"], "config");
    assert_eq!(listed[0]["vault_id"], app.vault.to_string());

    let (status, _) =
        json_request(&app, "DELETE", &format!("/api/sources/{missing_id}"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, bad) = json_request(
        &app,
        "POST",
        &format!("/api/sources/{missing_id}/test"),
        None,
    )
    .await;
    assert_eq!(bad["ok"], false);
    let (_, good) = json_request(&app, "POST", &format!("/api/sources/{ok_id}/test"), None).await;
    assert_eq!(good["ok"], true);
    std::fs::remove_dir_all(existing).unwrap();
}

#[tokio::test]
async fn ingest_workflows_take_only_skip_import() {
    let app = test_app(Vec::new()).await;
    let rule = json!({
        "name": "Ignore newsletters",
        "trigger": "ingest.received",
        "conditions": { "source": { "kind": "email", "email_from": { "contains": "newsletter" } } },
        "actions": [{ "type": "skip_import" }]
    });
    let (status, created) = json_request(&app, "POST", "/api/workflows", Some(rule.clone())).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        created["conditions"]["source"]["email_from"]["contains"],
        "newsletter"
    );

    let mut wrong_trigger = rule.clone();
    wrong_trigger["trigger"] = json!("document.uploaded");
    let (status, _) = json_request(&app, "POST", "/api/workflows", Some(wrong_trigger)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut with_webhook = rule.clone();
    with_webhook["actions"] = json!([{ "type": "webhook", "url": "http://localhost/x" }]);
    let (status, _) = json_request(&app, "POST", "/api/workflows", Some(with_webhook)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut bad_regex = rule;
    bad_regex["conditions"]["source"]["email_from"] = json!({ "regex": "(" });
    let (status, _) = json_request(&app, "POST", "/api/workflows", Some(bad_regex)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_sources_pages_render_and_save() {
    let app = test_app(vec![configured_folder("/consume")]).await;

    let (status, page) = send(&app, "GET", "/settings/sources", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("scans") && page.contains("/consume"));

    for uri in [
        "/settings/sources/new?kind=folder",
        "/settings/sources/new?kind=email",
    ] {
        assert_eq!(send(&app, "GET", uri, None).await.0, StatusCode::OK);
    }

    let response = app
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/sources")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "kind=email&name=Mail&enabled=on&host=imap.test&port=993&security=tls\
&username=me&password=pw&mailboxes=INBOX&poll_interval_secs=300&criteria=UNSEEN\
&allowed_content_types=application%2Fpdf&mail_body=none&mail_after_import=mark_read",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection(), "{}", response.status());

    let (_, listed) = json_request(&app, "GET", "/api/sources", None).await;
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert_eq!(listed[1]["config"]["password"], "********");
}
