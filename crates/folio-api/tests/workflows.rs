//! Workflow API and pages, driven through the real router.

use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    routing::post,
    Router,
};
use folio_api::{create_router, AppState, AuthState, IngestionState};
use folio_core::{
    models::{WorkflowAction, WorkflowRun, WorkflowTrigger},
    ports::DocumentStore,
    EventBroadcaster,
};
use folio_infra::{
    Config, Database, DynamicEmbeddingService, DynamicOcrService, ObjectStoreDocumentStore,
    PipelineContext, ThumbnailService, WebhookClient,
};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

struct TestApp {
    router: Router,
    state: AppState,
    vault: folio_core::models::VaultId,
    dir: std::path::PathBuf,
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn test_app() -> TestApp {
    test_app_with_auth(false).await
}

async fn test_app_with_auth(auth_required: bool) -> TestApp {
    let dir = std::env::temp_dir().join(format!("folio-workflows-api-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let storage_path = dir.to_str().unwrap().to_string();

    let database = Database::connect(&format!("sqlite:{storage_path}/test.db?mode=rwc"))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let repos = database.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let vaults = Arc::new(folio_core::services::VaultService::new(
        repos.vaults.clone(),
        repos.members,
    ));

    let auth = AuthState::new(repos.auth.clone(), None).with_required(auth_required);
    let store: Arc<dyn DocumentStore> = Arc::new(ObjectStoreDocumentStore::in_memory());
    let ai = Config::default().ai;
    let pipeline = PipelineContext::new(repos.settings.clone(), ai);
    let ingestion = IngestionState::new(
        repos.sources,
        repos.ledger,
        repos.ingest_outcomes,
        Vec::new(),
    );
    let state = AppState::new(
        repos.archival,
        repos.metadata_fields,
        repos.metadata_options,
        repos.metadata_values,
        repos.saved_views,
        repos.documents,
        repos.entries,
        repos.upload_receipts,
        repos.tags,
        repos.correspondents,
        repos.document_types,
        repos.settings.clone(),
        Arc::new(DynamicOcrService::new(pipeline.clone())),
        Arc::new(DynamicEmbeddingService::new(pipeline.clone())),
        repos.search,
        pipeline,
        Arc::new(ThumbnailService::new(store.clone())),
        repos.jobs,
        store,
        EventBroadcaster::default(),
        repos.outbox,
        repos.workflows,
        WebhookClient::new(),
        "http://folio.test".to_string(),
        ingestion,
        vaults,
        repos.access,
    );

    TestApp {
        router: create_router(state.clone(), auth),
        state,
        vault,
        dir,
    }
}

async fn send(app: &TestApp, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(json) => {
            builder = builder.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .router
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
    app: &TestApp,
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

async fn post_form(app: &TestApp, uri: &str, form: String) -> (StatusCode, String) {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(uri)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn workflow_body(url: &str) -> Value {
    json!({
        "name": "Notify n8n",
        "trigger": "document.uploaded",
        "conditions": { "mime_type": "application/pdf" },
        "actions": [{
            "type": "webhook",
            "url": url,
            "secret": "hunter2",
            "headers": { "X-Source": "folio" }
        }]
    })
}

#[tokio::test]
async fn condition_groups_survive_api_storage_and_html_edits() {
    let app = test_app().await;
    let groups = json!({
        "any": [{"filename_glob": "invoice-*"}, {"mime_type": "image/*"}],
        "none": [{"filename_glob": "draft-*"}]
    });
    let mut body = workflow_body("https://example.com/hook");
    body["conditions"] = groups.clone();
    let (status, created) = json_request(&app, "POST", "/api/workflows", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["conditions"], groups);
    let id = created["id"].as_str().unwrap();

    let response = app.router.clone().oneshot(
        Request::post(format!("/workflows/{id}"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("name=Grouped&trigger=document.uploaded&enabled=on&webhook_url=https%3A%2F%2Fexample.com%2Fhook"))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let (status, stored) = json_request(&app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored["name"], "Grouped");
    assert_eq!(stored["conditions"], groups);
}

#[tokio::test]
async fn web_editor_replaces_nested_groups_and_distinguishes_empty_any() {
    let app = test_app().await;
    let mut body = workflow_body("https://example.com/hook");
    body["conditions"] = json!({
        "mime_type": "application/pdf",
        "any": [{"filename_glob": "previous-*"}]
    });
    let (status, created) = json_request(&app, "POST", "/api/workflows", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap();
    let uri = format!("/workflows/{id}");
    let api = format!("/api/workflows/{id}");
    let submit = |groups: &Value| {
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("name", "Quarterly invoices")
            .append_pair("trigger", "document.uploaded")
            .append_pair("mime_type", "application/pdf")
            .append_pair("webhook_url", "https://example.com/hook")
            .append_pair("condition_groups", &groups.to_string())
            .finish()
    };
    let nested = json!({
        "all": [
            {
                "any": [{"filename_glob": "quarter-*"}, {"filename_glob": "invoice-*"}],
                "none": [{"filename_glob": "draft-*"}]
            }
        ],
        "any": []
    });
    let (status, _) = post_form(&app, &uri, submit(&nested)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (status, stored) = json_request(&app, "GET", &api, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        stored["conditions"],
        json!({
            "mime_type": "application/pdf",
            "all": nested["all"],
            "any": []
        })
    );

    let (status, _) = post_form(&app, &uri, submit(&json!({"all": nested["all"]}))).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (_, stored) = json_request(&app, "GET", &api, None).await;
    assert_eq!(
        stored["conditions"],
        json!({"mime_type": "application/pdf", "all": nested["all"]})
    );

    let bad = json!({"all": [{"source": {"email_from": {"regex": "("}}}]});
    let (status, html) = post_form(&app, &uri, submit(&bad)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Invalid regex"));
    let (_, unchanged) = json_request(&app, "GET", &api, None).await;
    assert_eq!(unchanged, stored);
}

/// A receiver that records requests and answers with a fixed status and diagnostic.
async fn receiver(status: u16) -> (String, Arc<Mutex<Vec<(HeaderMap, String)>>>) {
    type Log = Arc<Mutex<Vec<(HeaderMap, String)>>>;
    async fn handle(
        State((log, status)): State<(Log, u16)>,
        headers: HeaderMap,
        body: String,
    ) -> (StatusCode, &'static str) {
        log.lock().unwrap().push((headers, body));
        (
            StatusCode::from_u16(status).unwrap(),
            "Receiver diagnostic (fixture only)",
        )
    }

    let log: Log = Arc::default();
    let app = Router::new()
        .route("/hook", post(handle))
        .with_state((log.clone(), status));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, log)
}

#[tokio::test]
async fn crud_round_trip() {
    let app = test_app().await;

    let (status, created) = json_request(
        &app,
        "POST",
        "/api/workflows",
        Some(workflow_body("http://192.168.1.20:5678/webhook/folio")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["name"], "Notify n8n");
    assert_eq!(created["enabled"], true);
    assert_eq!(created["trigger"], "document.uploaded");
    assert_eq!(
        created["actions"][0]["secret"], "********",
        "secrets are masked"
    );

    let (status, listed) = json_request(&app, "GET", "/api/workflows", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1);

    let (status, fetched) = json_request(&app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["conditions"]["mime_type"], "application/pdf");

    // Replacing with the masked secret keeps the stored one.
    let mut update = workflow_body("http://192.168.1.20:5678/webhook/folio");
    update["name"] = json!("Renamed");
    update["enabled"] = json!(false);
    update["actions"][0]["secret"] = json!("********");
    let (status, updated) =
        json_request(&app, "PUT", &format!("/api/workflows/{id}"), Some(update)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["name"], "Renamed");
    assert_eq!(updated["enabled"], false);

    let stored = app
        .state
        .workflow_repo
        .find_by_id(app.vault, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let WorkflowAction::Webhook { secret, .. } = &stored.actions[0] else {
        panic!("expected a webhook");
    };
    assert_eq!(secret.as_deref(), Some("hunter2"));

    let (status, _) = json_request(&app, "DELETE", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = json_request(&app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rejects_non_http_urls_and_empty_workflows() {
    let app = test_app().await;

    for url in ["file:///etc/passwd", "ftp://example.com/x", "not a url"] {
        let (status, body) =
            json_request(&app, "POST", "/api/workflows", Some(workflow_body(url))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{url}");
        assert!(body["error"].is_string());
    }

    let mut no_actions = workflow_body("http://localhost/x");
    no_actions["actions"] = json!([]);
    let (status, _) = json_request(&app, "POST", "/api/workflows", Some(no_actions)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_endpoint_sends_a_signed_sample_payload() {
    let app = test_app().await;
    let (url, log) = receiver(200).await;
    let (_, created) =
        json_request(&app, "POST", "/api/workflows", Some(workflow_body(&url))).await;
    let id = created["id"].as_str().unwrap();

    let (status, result) =
        json_request(&app, "POST", &format!("/api/workflows/{id}/test"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["deliveries"][0]["success"], true);
    assert_eq!(result["deliveries"][0]["status"], 200);
    assert_eq!(result["payload"]["event"], "document.uploaded");
    assert_eq!(result["payload"]["version"], 1);

    let log = log.lock().unwrap();
    assert_eq!(log.len(), 1);
    let (headers, body) = &log[0];
    let timestamp: i64 = headers["x-folio-timestamp"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        headers["x-folio-signature"].to_str().unwrap(),
        folio_infra::workflow::sign("hunter2", timestamp, body)
    );
    assert_eq!(headers["x-source"], "folio");
    let sent: Value = serde_json::from_str(body).unwrap();
    assert_eq!(sent["event"], "document.uploaded");
    assert_eq!(
        sent["document"]["urls"]["api"]
            .as_str()
            .unwrap()
            .split("/api")
            .next(),
        Some("http://folio.test")
    );
}

#[tokio::test]
async fn test_endpoint_reports_a_failing_receiver() {
    let app = test_app().await;
    let (url, _log) = receiver(500).await;
    let (_, created) =
        json_request(&app, "POST", "/api/workflows", Some(workflow_body(&url))).await;
    let id = created["id"].as_str().unwrap();

    let (status, result) =
        json_request(&app, "POST", &format!("/api/workflows/{id}/test"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["deliveries"][0]["success"], false);
    assert_eq!(result["deliveries"][0]["status"], 500);
    assert_eq!(result["deliveries"][0]["error"], "HTTP 500");
}

#[tokio::test]
async fn test_endpoint_needs_a_webhook() {
    let app = test_app().await;
    let mut body = workflow_body("http://localhost/x");
    body["actions"] = json!([{ "type": "add_tags", "tag_ids": [Uuid::new_v4()] }]);
    let (_, created) = json_request(&app, "POST", "/api/workflows", Some(body)).await;
    let id = created["id"].as_str().unwrap();

    let (status, _) = json_request(&app, "POST", &format!("/api/workflows/{id}/test"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_form_tests_unsaved_edits_and_keeps_them() {
    let app = test_app().await;
    let (url, log) = receiver(200).await;
    let mut body = workflow_body(&url);
    body["actions"][0]["headers"] = json!({ "Authorization": "Bearer abc" });
    let (_, created) = json_request(&app, "POST", "/api/workflows", Some(body)).await;
    let id = created["id"].as_str().unwrap();

    let test_uri = format!("/workflows/{id}/test");
    let draft = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("name", "Draft name")
        .append_pair("trigger", "document.processed")
        .append_pair("enabled", "on")
        .append_pair("webhook_url", &url)
        .append_pair("webhook_headers", "Authorization: ********\nX-Draft: yes")
        .finish();
    let (status, html) = post_form(&app, &test_uri, draft).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Delivered to"), "{html}");
    assert!(html.contains("Draft name"), "the edits stay in the form");
    assert!(html.contains("X-Draft: yes"));

    {
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 1);
        let (headers, body) = &log[0];
        assert_eq!(headers["x-draft"], "yes", "the draft is what gets sent");
        assert_eq!(
            headers["authorization"], "Bearer abc",
            "masked values stand for stored ones"
        );
        let timestamp: i64 = headers["x-folio-timestamp"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            headers["x-folio-signature"].to_str().unwrap(),
            folio_infra::workflow::sign("hunter2", timestamp, body),
            "the stored secret signs the draft"
        );
        let sent: Value = serde_json::from_str(body).unwrap();
        assert_eq!(sent["event"], "document.processed");
    }

    let stored = app
        .state
        .workflow_repo
        .find_by_id(app.vault, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.name, "Notify n8n", "testing does not save");

    let (status, html) = post_form(&app, &test_uri, "name=Broken&trigger=nope".into()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Choose a trigger."));
    assert!(html.contains("Broken"));
    assert_eq!(log.lock().unwrap().len(), 1, "an invalid draft is not sent");
}

#[tokio::test]
async fn lists_runs_newest_first() {
    let app = test_app().await;
    let (_, created) = json_request(
        &app,
        "POST",
        "/api/workflows",
        Some(workflow_body("http://localhost/x")),
    )
    .await;
    let id: Uuid = created["id"].as_str().unwrap().parse().unwrap();

    for _ in 0..3 {
        let run = WorkflowRun::new(
            created["vault_id"].as_str().unwrap().parse().unwrap(),
            id,
            WorkflowTrigger::DocumentUploaded,
            None,
            "{}".into(),
        );
        app.state.workflow_repo.create_run(&run).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }

    let (status, runs) =
        json_request(&app, "GET", &format!("/api/workflows/{id}/runs"), None).await;
    assert_eq!(status, StatusCode::OK);
    let runs = runs.as_array().unwrap();
    assert_eq!(runs.len(), 3);
    assert!(runs[0]["created_at"].as_str() >= runs[2]["created_at"].as_str());
    assert_eq!(runs[0]["status"], "pending");
    assert!(runs[0].get("payload").is_none());

    let (_, limited) = json_request(
        &app,
        "GET",
        &format!("/api/workflows/{id}/runs?limit=1"),
        None,
    )
    .await;
    assert_eq!(limited.as_array().unwrap().len(), 1);

    let (status, _) = json_request(
        &app,
        "GET",
        &format!("/api/workflows/{}/runs", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn run_history_preserves_progress_without_receiver_diagnostics() {
    let app = test_app().await;
    let input = serde_json::from_value(workflow_body("http://127.0.0.1:1/unused")).unwrap();
    let workflow = app
        .state
        .workflow_repo
        .create(app.vault, input)
        .await
        .unwrap();
    let id = workflow.id;
    let mut run = WorkflowRun::new(
        app.vault,
        id,
        workflow.trigger,
        Some(Uuid::new_v4()),
        r#"{"note":"Captured payload (fixture only)"}"#.into(),
    )
    .with_actions(workflow.actions);
    run.attempts = 2;
    run.completed_actions = 1;
    run.response_status = Some(503);
    run.last_error =
        Some("HTTP 503 Service Unavailable: Receiver diagnostic (fixture only)".into());
    assert!(app.state.workflow_repo.create_run(&run).await.unwrap());

    let (status, runs) =
        json_request(&app, "GET", &format!("/api/workflows/{id}/runs"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(runs[0]["last_error"], "HTTP 503");
    assert_eq!(runs[0]["response_status"], 503);
    assert_eq!(runs[0]["completed_actions"], 1);
    assert!(runs[0].get("payload").is_none());
    assert!(runs[0].get("actions").is_none());

    for uri in [
        format!("/workflows/{id}/history"),
        format!("/workflows/{id}"),
    ] {
        let (status, html) = send(&app, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("Retrying"));
        assert!(html.contains("<dt>Completed actions</dt><dd>1</dd>"));
        assert!(!html.contains("Receiver diagnostic (fixture only)"));
        assert!(!html.contains("Captured payload (fixture only)"));
    }
}

#[tokio::test]
async fn pages_render_and_the_form_creates_a_workflow() {
    let app = test_app().await;

    let (status, html) = send(&app, "GET", "/workflows", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("No workflows yet"));

    let (status, html) = send(&app, "GET", "/workflows/new", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("document.processed"));
    for kind in ["note", "comment"] {
        for transition in ["created", "updated", "deleted"] {
            assert!(html.contains(&format!("document_{kind}.{transition}")));
        }
    }

    let form = "name=Home+Assistant&trigger=document.uploaded&enabled=on&mime_type=application%2Fpdf\
                &webhook_url=http%3A%2F%2F192.168.1.5%3A8123%2Fhook&webhook_secret=s3&webhook_headers=X-A%3A+b";
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/workflows")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()["location"].to_str().unwrap().to_string();

    let (status, html) = send(&app, "GET", &location, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Home Assistant"));
    assert!(html.contains("http://192.168.1.5:8123/hook"));
    assert!(!html.contains("s3\""), "the secret is never rendered back");
    assert!(html.contains("Send test"));

    let (_, html) = send(&app, "GET", "/workflows", None).await;
    assert!(html.contains("Home Assistant"));
    assert!(html.contains("No runs yet"));
}

#[tokio::test]
async fn precise_metadata_trigger_roundtrips_through_api_and_web_editor() {
    let app = test_app().await;
    let mut body = workflow_body("https://example.com/hook");
    body["trigger"] = json!("document.tags_changed");
    let (status, created) = json_request(&app, "POST", "/api/workflows", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["trigger"], "document.tags_changed");
    let id = created["id"].as_str().unwrap();
    let (status, html) = send(&app, "GET", &format!("/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("value=\"document.tags_changed\" selected"));
    assert!(html.contains("When document tags change"));
}

#[tokio::test]
async fn web_edit_preserves_processing_predicates_at_the_root_and_in_groups() {
    let app = test_app().await;
    let trigger = "document.processing.ocr.completed";
    let root = json!({"processing": {"has_text": true}});
    let groups = json!({
        "all": [{"event": {"processing": {"attempt": 2}}, "filename_glob": "invoice-*"}]
    });
    let mut body = workflow_body("https://example.com/hook");
    body["trigger"] = json!(trigger);
    body["conditions"] = json!({"event": root, "all": groups["all"]});
    let (status, created) = json_request(&app, "POST", "/api/workflows", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap();
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("name", "Reviewed extraction")
        .append_pair("trigger", trigger)
        .append_pair("event_actor", "")
        .append_pair("webhook_url", "https://example.com/hook")
        .append_pair("condition_groups", &groups.to_string())
        .finish();

    let (status, html) = post_form(&app, &format!("/workflows/{id}"), form).await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{html}");
    let (status, saved) = json_request(&app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["name"], "Reviewed extraction");
    assert_eq!(saved["conditions"], created["conditions"]);
}

#[tokio::test]
async fn lifecycle_trigger_editor_explains_when_document_actions_are_unavailable() {
    let app = test_app().await;
    let (status, tag) =
        json_request(&app, "POST", "/api/tags", Some(json!({"name": "restored"}))).await;
    assert_eq!(status, StatusCode::OK);
    let tag = tag["id"].as_str().unwrap();
    let (status, created) = json_request(
        &app,
        "POST",
        "/api/workflows",
        Some(json!({
            "name": "Tag after restore",
            "trigger": "document.restored",
            "actions": [{"type": "add_tags", "tag_ids": [tag]}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let workflow = created["id"].as_str().unwrap();
    let edit_url = format!("/workflows/{workflow}");
    let (status, html) = send(&app, "GET", &edit_url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("value=\"document.restored\" selected"));
    assert!(html.contains("When a document is restored"));

    let form =
        format!("name=Tag+after+restore&trigger=document.moved_out&enabled=on&action_tag_{tag}=on");
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(edit_url)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body)
        .contains("Only webhook actions can run for this document lifecycle event"));
    let (status, stored) =
        json_request(&app, "GET", &format!("/api/workflows/{workflow}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored["trigger"], "document.restored");
}

#[tokio::test]
async fn the_form_explains_rejected_input() {
    let app = test_app().await;
    let form = "name=Bad&trigger=document.uploaded&webhook_url=ftp%3A%2F%2Fexample.com";
    let (status, html) = {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::post("/workflows")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    };
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("must use http or https"));
}

#[tokio::test]
async fn header_values_are_masked_and_kept_on_update() {
    let app = test_app().await;
    let mut body = workflow_body("http://localhost:5678/hook");
    body["actions"][0]["headers"] = json!({ "Authorization": "Bearer abc" });

    let (_, created) = json_request(&app, "POST", "/api/workflows", Some(body.clone())).await;
    assert_eq!(
        created["actions"][0]["headers"]["Authorization"],
        "********"
    );
    let id = created["id"].as_str().unwrap();

    body["actions"][0]["headers"] = json!({ "Authorization": "********" });
    body["actions"][0]["secret"] = json!("********");
    let (status, _) = json_request(
        &app,
        "PUT",
        &format!("/api/workflows/{id}"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let stored = app
        .state
        .workflow_repo
        .find_by_id(app.vault, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let WorkflowAction::Webhook {
        headers, secret, ..
    } = &stored.actions[0]
    else {
        panic!("expected a webhook");
    };
    assert_eq!(headers["Authorization"], "Bearer abc");
    assert_eq!(secret.as_deref(), Some("hunter2"));

    // Pointing the webhook elsewhere must not carry stored values along.
    body["actions"][0]["url"] = json!("http://evil.example/hook");
    let (status, _) = json_request(&app, "PUT", &format!("/api/workflows/{id}"), Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_rejects_the_mask_and_errors_hide_credentials() {
    let app = test_app().await;
    let mut body = workflow_body("http://localhost/x");
    body["actions"][0]["secret"] = json!("********");
    let (status, _) = json_request(&app, "POST", "/api/workflows", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, error) = json_request(
        &app,
        "POST",
        "/api/workflows",
        Some(workflow_body("ftp://user:pw@example.com/x")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!error.to_string().contains("pw"));
}

#[tokio::test]
async fn workflows_sit_behind_authentication() {
    let app = test_app_with_auth(true).await;

    let (status, _) = send(&app, "GET", "/api/workflows", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(
        &app,
        "POST",
        "/api/workflows",
        Some(workflow_body("http://localhost/x")),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Pages redirect to login instead of rendering
    let (status, _) = send(&app, "GET", "/workflows", None).await;
    assert!(
        status == StatusCode::SEE_OTHER || status == StatusCode::UNAUTHORIZED,
        "unexpected status {status}"
    );
}

#[tokio::test]
async fn the_form_shows_and_edits_every_action() {
    let app = test_app().await;
    let (_, tag) = json_request(
        &app,
        "POST",
        "/api/tags",
        Some(json!({ "name": "Utilities" })),
    )
    .await;
    let (_, sender) = json_request(
        &app,
        "POST",
        "/api/correspondents",
        Some(json!({ "name": "Stadtwerke" })),
    )
    .await;
    let (_, created) = json_request(
        &app,
        "POST",
        "/api/workflows",
        Some(json!({
            "name": "File power bills",
            "trigger": "document.processed",
            "actions": [
                { "type": "set_correspondent", "correspondent_id": sender["id"] },
                { "type": "add_tags", "tag_ids": [tag["id"]] }
            ]
        })),
    )
    .await;
    let id = created["id"].as_str().unwrap();
    let tag_id = tag["id"].as_str().unwrap();
    let sender_id = sender["id"].as_str().unwrap();

    let (status, html) = send(&app, "GET", &format!("/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("When a document has been read"));
    assert!(html.contains(&format!(r#"name="action_tag_{tag_id}" checked"#)));
    assert!(html.contains(&format!(r#"value="{sender_id}" selected"#)));
    assert!(html.contains(r#"aria-label="How to match the sender""#));
    assert!(html.contains(r#"for="email_to""#));

    // Saving without a webhook keeps the other actions and drops the cleared one
    let form = format!(
        "name=File+power+bills&trigger=document.processed&enabled=on&action_tag_{tag_id}=on\
         &action_correspondent_id=&action_document_type_id="
    );
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/workflows/{id}"))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let stored = app
        .state
        .workflow_repo
        .find_by_id(app.vault, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.actions.len(), 1);
    assert!(
        matches!(&stored.actions[0], WorkflowAction::AddTags { tag_ids } if tag_ids.len() == 1)
    );
}

#[tokio::test]
async fn workflow_management_pages_show_retained_actor_attribution_as_text() {
    use folio_core::{models::UserId, mutation::with_actor};

    let app = test_app().await;
    let input = serde_json::from_value(workflow_body("http://127.0.0.1:1/unused")).unwrap();
    let workflow = with_actor(
        Some(UserId::from("creator<script>")),
        app.state.workflow_repo.create(app.vault, input),
    )
    .await
    .unwrap();
    let mut edited = workflow_body("http://127.0.0.1:1/unused");
    edited["name"] = json!("Edited workflow");
    with_actor(
        Some(UserId::from("editor")),
        app.state.workflow_repo.update(
            app.vault,
            workflow.id,
            serde_json::from_value(edited).unwrap(),
        ),
    )
    .await
    .unwrap();

    let (status, active) = send(&app, "GET", "/workflows", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        active.contains("by creator&lt;script&gt;")
            || active.contains("by creator&#60;script&#62;")
    );
    assert!(active.contains("by editor"));
    assert!(!active.contains("by creator<script>"));
    let history_url = format!("/workflows/{}/history", workflow.id);
    let (status, history) = send(&app, "GET", &history_url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        history.contains("by creator&lt;script&gt;")
            || history.contains("by creator&#60;script&#62;")
    );
    assert!(history.contains("by editor"));

    with_actor(
        Some(UserId::from("remover")),
        app.state.workflow_repo.delete(app.vault, workflow.id),
    )
    .await
    .unwrap();
    let (status, trash) = send(&app, "GET", "/workflows?deleted=true", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        trash.contains("by creator&lt;script&gt;") || trash.contains("by creator&#60;script&#62;")
    );
    assert!(trash.contains("by remover"));
    let (status, history) = send(&app, "GET", &history_url, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        history.contains("by creator&lt;script&gt;")
            || history.contains("by creator&#60;script&#62;")
    );
    assert!(history.contains("by remover"));
    assert!(history.contains(", deleted "));
}

#[tokio::test]
async fn workflow_management_pages_label_unattributed_changes() {
    let app = test_app().await;
    let (_, created) = json_request(
        &app,
        "POST",
        "/api/workflows",
        Some(workflow_body("http://127.0.0.1:1/unused")),
    )
    .await;
    let id = created["id"].as_str().unwrap();
    let (status, page) = send(&app, "GET", "/workflows", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page.matches("by System or unknown").count(), 2);
    let (status, history) = send(&app, "GET", &format!("/workflows/{id}/history"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(history.matches("by System or unknown").count(), 2);
}

#[tokio::test]
async fn workflow_trash_restore_and_retained_history_are_usable_from_management_pages() {
    let app = test_app().await;
    let mut input = workflow_body("http://127.0.0.1:1/unused");
    input["created_by"] = json!("forged");
    input["updated_by"] = json!("forged");
    let (status, created) = json_request(&app, "POST", "/api/workflows", Some(input)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(created["created_by"].is_null());
    assert!(created["updated_by"].is_null());
    let id = created["id"].as_str().unwrap();
    let mut masked = workflow_body("http://127.0.0.1:1/unused");
    masked["actions"][0]["secret"] = json!("********");
    masked["actions"][0]["headers"]["X-Source"] = json!("********");
    let (status, unchanged) =
        json_request(&app, "PUT", &format!("/api/workflows/{id}"), Some(masked)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unchanged["updated_at"], created["updated_at"]);
    let stored = app
        .state
        .workflow_repo
        .find_by_id(app.vault, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let mut run = WorkflowRun::new(
        app.vault,
        stored.id,
        stored.trigger,
        Some(Uuid::new_v4()),
        r#"{"note":"frozen execution context"}"#.into(),
    )
    .with_actions(stored.actions.clone());
    run.response_status = Some(503);
    app.state.workflow_repo.create_run(&run).await.unwrap();
    let (status, _) = send(&app, "POST", &format!("/workflows/{id}/delete"), None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (status, _) = json_request(&app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, deleted) = json_request(&app, "GET", "/api/workflows?deleted=true", None).await;
    assert_eq!(deleted.as_array().unwrap().len(), 1);
    assert!(deleted[0]["deleted_at"].is_string());
    assert_eq!(deleted[0]["actions"][0]["secret"], "********");
    let (status, page) = send(&app, "GET", "/workflows?deleted=true", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(&format!("/{id}/restore")));
    assert!(page.contains(&format!("/{id}/history")));
    let (status, history) = send(&app, "GET", &format!("/workflows/{id}/history"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(history.contains("Workflow was deleted"));
    assert!(history.contains("<dt>Finished</dt>"));
    assert!(!history.contains("hunter2"));
    let (status, runs) =
        json_request(&app, "GET", &format!("/api/workflows/{id}/runs"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(runs[0]["status"], "failed");
    assert_eq!(runs[0]["last_error"], "Workflow was deleted");
    let (status, _) = send(&app, "POST", &format!("/workflows/{id}/restore"), None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (_, restored) = json_request(&app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(restored["created_at"], created["created_at"]);
    assert!(restored["deleted_at"].is_null());
    let (status, unchanged) =
        json_request(&app, "POST", &format!("/api/workflows/{id}/restore"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unchanged["updated_at"], restored["updated_at"]);
    let (_, retained_runs) =
        json_request(&app, "GET", &format!("/api/workflows/{id}/runs"), None).await;
    assert_eq!(retained_runs[0]["status"], "failed");
}

#[tokio::test]
async fn typed_event_conditions_survive_api_storage_and_html_edits() {
    let app = test_app().await;
    let conditions = json!({"event": {
        "subject_id": Uuid::new_v4(), "actor": {"type": "user", "user_id": "editor"}
    }});
    let mut body = workflow_body("https://example.com/hook");
    body["conditions"] = conditions.clone();
    let (status, created) = json_request(&app, "POST", "/api/workflows", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["conditions"], conditions);
    let id = created["id"].as_str().unwrap();
    let response = app.router.clone().oneshot(
        Request::post(format!("/workflows/{id}"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("name=Event+filter&trigger=document.uploaded&enabled=on&webhook_url=https%3A%2F%2Fexample.com%2Fhook"))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let (status, stored) = json_request(&app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored["conditions"], conditions);
}

#[tokio::test]
async fn web_editor_creates_an_entry_workflow_with_occurrence_filters() {
    let app = test_app().await;
    let subject = Uuid::new_v4();
    let form = format!(
        "name=Comment+alert&trigger=document_comment.updated&enabled=on&event_actor=user&event_user_id=editor&event_subject_id={subject}&webhook_url=https%3A%2F%2Fexample.com%2Fhook"
    );
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/workflows")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let id = response.headers()["location"]
        .to_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap();
    let (status, stored) = json_request(&app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored["trigger"], "document_comment.updated");
    assert_eq!(
        stored["conditions"]["event"],
        json!({
            "subject_id": subject, "actor": {"type": "user", "user_id": "editor"}
        })
    );
}

#[tokio::test]
async fn trigger_catalog_matches_workflow_choices_and_describes_entry_filters() {
    let app = test_app().await;
    let (status, json) = json_request(&app, "GET", "/api/workflows/triggers", None).await;
    assert_eq!(status, StatusCode::OK);
    let catalog = json.as_array().unwrap();
    let (status, html) = send(&app, "GET", "/workflows/new", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(catalog.len(), WorkflowTrigger::ALL.len());
    for choice in catalog {
        let event_type = choice["event_type"].as_str().unwrap();
        assert!(html.contains(&format!(r#"value="{event_type}""#)));
        assert!(WorkflowTrigger::parse(event_type).is_some());
        if event_type.starts_with("document_note.") || event_type.starts_with("document_comment.") {
            assert_eq!(choice["target"], "document");
            assert!(choice["condition_fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field["key"] == "event.subject_id"));
        }
    }
}

#[tokio::test]
async fn trigger_catalog_describes_only_fields_backed_by_the_occurrence() {
    let app = test_app().await;
    let (status, json) = json_request(&app, "GET", "/api/workflows/triggers", None).await;
    assert_eq!(status, StatusCode::OK);
    let catalog = json.as_array().unwrap();
    let field_type = |trigger: &str, key: &str| {
        catalog
            .iter()
            .find(|choice| choice["event_type"] == trigger)
            .unwrap()["condition_fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["key"] == key)
            .and_then(|field| field["value_type"].as_str())
    };

    let email_fields = [
        "source.email_from",
        "source.email_to",
        "source.email_subject",
    ];
    for trigger in [
        "document.uploaded",
        "document.title_changed",
        "document.deleted",
    ] {
        assert_eq!(field_type(trigger, "mime_type"), Some("text"));
        assert_eq!(field_type(trigger, "filename_glob"), Some("text"));
        assert_eq!(field_type(trigger, "source.kind"), Some("text"));
        for field in email_fields {
            assert_eq!(field_type(trigger, field), Some("text"));
        }
        assert_eq!(field_type(trigger, "source.folder_path_glob"), Some("text"));
        assert_eq!(
            field_type(trigger, "source.email_attachment_glob"),
            Some("text")
        );
        assert_eq!(
            field_type(trigger, "source.email_mailbox_glob"),
            Some("text")
        );
        assert_eq!(field_type(trigger, "tag_id"), Some("identifier"));
        assert_eq!(field_type(trigger, "correspondent_id"), Some("identifier"));
        assert_eq!(field_type(trigger, "document_type_id"), Some("identifier"));
    }
    assert_eq!(field_type("ingest.received", "filename_glob"), Some("text"));
    assert_eq!(field_type("ingest.received", "source.kind"), Some("text"));
    for field in email_fields {
        assert_eq!(field_type("ingest.received", field), Some("text"));
    }
    assert_eq!(field_type("ingest.received", "tag_id"), None);
    assert_eq!(field_type("ingest.received", "event.subject_id"), None);

    for trigger in [
        "document_note.created",
        "document_comment.updated",
        "document.metadata.changed",
        "ingest.failed",
        "source.recovered",
    ] {
        assert_eq!(field_type(trigger, "mime_type"), None);
        assert_eq!(field_type(trigger, "tag_id"), None);
        assert_eq!(field_type(trigger, "source.folder_path_glob"), None);
        assert_eq!(field_type(trigger, "source.kind"), None);
        for field in email_fields {
            assert_eq!(field_type(trigger, field), None);
        }
    }
    assert_eq!(
        field_type("document_note.created", "event.subject_id"),
        Some("identifier")
    );
    assert_eq!(
        field_type("document.metadata.changed", "event.metadata.after"),
        Some("metadata_value")
    );
    assert_eq!(
        field_type("ingest.failed", "data.retryable"),
        Some("boolean")
    );
}

#[tokio::test]
async fn web_editor_configures_typed_ingestion_outcome_filters() {
    let app = test_app().await;
    let source = Uuid::new_v4();
    let form = format!(
        "name=Failed+poll&trigger=ingest.failed&enabled=on&event_actor=&ingest_source_id={source}&ingest_source_kind=folder&ingest_retryable=true&webhook_url=https%3A%2F%2Fexample.com%2Fhook"
    );
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/workflows")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let id = response.headers()["location"]
        .to_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap();
    let (status, stored) = json_request(&app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        stored["conditions"]["event"]["ingest"],
        json!({
            "source_id": source,
            "source_kind": "folder",
            "retryable": true,
        })
    );
    assert_eq!(stored["actions"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn web_editor_configures_exact_custom_metadata_conditions() {
    let app = test_app().await;
    let field = Uuid::new_v4();
    let form = format!(
        "name=Exact+amount&trigger=document.metadata.changed&enabled=on&event_actor=&metadata_field_id={field}&metadata_kind=decimal&metadata_state=set&metadata_after_enabled=on&metadata_after=9007199254740993.0100&webhook_url=https%3A%2F%2Fexample.com%2Fhook"
    );
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/workflows")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let id = response.headers()["location"]
        .to_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap();
    let (status, stored) = json_request(&app, "GET", &format!("/api/workflows/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        stored["conditions"]["event"]["metadata"],
        json!({
            "field_id": field, "kind": "decimal", "state": "set",
            "after": {"kind": "decimal", "value": "9007199254740993.01"},
        })
    );
}

#[tokio::test]
async fn web_form_authors_and_validates_typed_processing_filters() {
    let app = test_app().await;
    let (status, created) = json_request(
        &app,
        "POST",
        "/api/workflows",
        Some(json!({
            "name": "Retry capture",
            "trigger": "document.processing.ocr.retry_scheduled",
            "actions": [{"type": "webhook", "url": "https://example.test/ocr"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap();
    let uri = format!("/workflows/{id}");
    let run = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let retry_at = "2040-01-03T09:00:00.123456789Z";
    let form = |trigger: &str, attempt: &str| {
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("name", "Retry capture")
            .append_pair("trigger", trigger)
            .append_pair("event_actor", "")
            .append_pair("processing_form", "1")
            .append_pair("processing_stage", "ocr")
            .append_pair("processing_outcome", "retry_scheduled")
            .append_pair("processing_run_id", &run.to_string())
            .append_pair("processing_revision_id", &revision.to_string())
            .append_pair("processing_attempt", attempt)
            .append_pair("processing_reason_code", "provider_failed")
            .append_pair("processing_retry_at", retry_at)
            .append_pair("webhook_url", "https://example.test/ocr")
            .finish()
    };
    let trigger = "document.processing.ocr.retry_scheduled";
    let (saved, _) = post_form(&app, &uri, form(trigger, "3")).await;
    assert_eq!(saved, StatusCode::SEE_OTHER);
    let (_, stored) = json_request(&app, "GET", &format!("/api{uri}"), None).await;
    assert_eq!(
        stored["conditions"]["event"]["processing"],
        json!({
            "stage": "ocr", "outcome": "retry_scheduled", "run_id": run, "revision_id": revision,
            "attempt": 3, "reason_code": "provider_failed", "retry_at": retry_at,
        })
    );
    let (_, html) = send(&app, "GET", &uri, None).await;
    assert!(html.contains(r#"name="processing_attempt" value="3""#));
    assert!(html.contains(retry_at));

    let (invalid, html) = post_form(&app, &uri, form(trigger, "4294967296")).await;
    assert_eq!(invalid, StatusCode::OK);
    assert!(html.contains("processing attempt"));
    let (invalid, html) =
        post_form(&app, &uri, form("document.processing.ocr.completed", "3")).await;
    assert_eq!(invalid, StatusCode::OK);
    assert!(html.contains("These processing conditions are unavailable for the selected trigger"));
    let (_, unchanged) = json_request(&app, "GET", &format!("/api{uri}"), None).await;
    assert_eq!(unchanged["conditions"], stored["conditions"]);
    assert_eq!(unchanged["trigger"], stored["trigger"]);
}
