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
    let ingestion = IngestionState::new(repos.sources, repos.ledger, Vec::new());
    let state = AppState::new(
        repos.documents,
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

/// A receiver that records requests and answers with a fixed status
async fn receiver(status: u16) -> (String, Arc<Mutex<Vec<(HeaderMap, String)>>>) {
    type Log = Arc<Mutex<Vec<(HeaderMap, String)>>>;
    async fn handle(
        State((log, status)): State<(Log, u16)>,
        headers: HeaderMap,
        body: String,
    ) -> StatusCode {
        log.lock().unwrap().push((headers, body));
        StatusCode::from_u16(status).unwrap()
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
    assert!(result["deliveries"][0]["error"]
        .as_str()
        .unwrap()
        .contains("500"));
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
async fn pages_render_and_the_form_creates_a_workflow() {
    let app = test_app().await;

    let (status, html) = send(&app, "GET", "/workflows", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("No workflows yet"));

    let (status, html) = send(&app, "GET", "/workflows/new", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("document.processed"));

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
