//! Workflow engine and webhook delivery against real SQLite and a local HTTP server.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use tokio::sync::watch;
use uuid::Uuid;

use folio_core::models::{
    CreateCorrespondent, CreateDocument, CreateTag, Document, Id, RunStatus, UpdateDocument,
    WebhookMethod, Workflow, WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun,
    WorkflowTrigger,
};
use folio_core::ports::{
    CorrespondentRepository, DocumentRepository, TagRepository, WorkflowRepository,
};
use folio_core::services::{EventingDocumentRepository, WorkflowEngine};
use folio_core::{DocumentEvent, EventBroadcaster};
use folio_infra::workflow::{sign, RunOutcome, WebhookRequest, MAX_ATTEMPTS};
use folio_infra::{
    create_pool, run_migrations, FileStorage, JobQueue, SqliteCorrespondentRepository,
    SqliteDocumentRepository, SqliteDocumentTypeRepository, SqliteTagRepository,
    SqliteWorkflowRepository, StorageFileMetadata, WebhookClient, WorkflowRunner,
};

struct Received {
    path: String,
    headers: HeaderMap,
    body: String,
}

#[derive(Default)]
struct ServerState {
    received: Mutex<Vec<Received>>,
    /// Status codes to answer with per path, consumed in order; 200 once empty
    script: Mutex<HashMap<String, VecDeque<u16>>>,
}

struct TestServer {
    base: String,
    state: Arc<ServerState>,
}

impl TestServer {
    async fn start() -> Self {
        async fn handler(
            State(state): State<Arc<ServerState>>,
            Path(path): Path<String>,
            headers: HeaderMap,
            body: String,
        ) -> StatusCode {
            let status = state
                .script
                .lock()
                .unwrap()
                .get_mut(&path)
                .and_then(|q| q.pop_front())
                .unwrap_or(200);
            state.received.lock().unwrap().push(Received {
                path,
                headers,
                body,
            });
            StatusCode::from_u16(status).unwrap()
        }

        let state = Arc::new(ServerState::default());
        let app = Router::new()
            .route("/{*path}", post(handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { base, state }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base, path)
    }

    fn script(&self, path: &str, statuses: &[u16]) {
        self.state
            .script
            .lock()
            .unwrap()
            .insert(path.to_string(), statuses.iter().copied().collect());
    }

    fn hits(&self, path: &str) -> usize {
        self.state
            .received
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path == path)
            .count()
    }
}

struct Harness {
    workflows: Arc<SqliteWorkflowRepository>,
    raw_documents: Arc<SqliteDocumentRepository>,
    documents: Arc<EventingDocumentRepository>,
    tags: Arc<SqliteTagRepository>,
    correspondents: Arc<SqliteCorrespondentRepository>,
    queue: JobQueue,
    events: EventBroadcaster,
    engine: Arc<WorkflowEngine>,
    runner: WorkflowRunner,
    dir: std::path::PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Harness {
    async fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("folio-workflows-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pool = create_pool(&format!("sqlite://{}/test.db", dir.display()))
            .await
            .unwrap();
        run_migrations(&pool).await.unwrap();

        let events = EventBroadcaster::default();
        let workflows = Arc::new(SqliteWorkflowRepository::new(pool.clone()));
        let raw_documents = Arc::new(SqliteDocumentRepository::new(pool.clone()));
        let documents = Arc::new(EventingDocumentRepository::new(
            raw_documents.clone(),
            events.clone(),
        ));
        let tags = Arc::new(SqliteTagRepository::new(pool.clone()));
        let correspondents = Arc::new(SqliteCorrespondentRepository::new(pool.clone()));
        let queue = JobQueue::new(pool.clone());

        let engine = Arc::new(
            WorkflowEngine::new(
                workflows.clone(),
                raw_documents.clone(),
                correspondents.clone(),
                Arc::new(SqliteDocumentTypeRepository::new(pool.clone())),
                Arc::new(queue.clone()),
                Arc::new(StorageFileMetadata::new(FileStorage::new(&dir))),
                "http://folio.test",
            )
            .with_file_wait(Duration::ZERO),
        );
        let runner = WorkflowRunner::new(
            workflows.clone(),
            raw_documents.clone(),
            WebhookClient::new(),
        );

        Self {
            workflows,
            raw_documents,
            documents,
            tags,
            correspondents,
            queue,
            events,
            engine,
            runner,
            dir,
        }
    }

    async fn workflow(
        &self,
        trigger: WorkflowTrigger,
        conditions: WorkflowConditions,
        actions: Vec<WorkflowAction>,
    ) -> Workflow {
        self.workflows
            .create(WorkflowInput {
                name: "test".into(),
                enabled: true,
                trigger,
                conditions,
                actions,
            })
            .await
            .unwrap()
    }

    async fn webhook_workflow(&self, url: String, secret: Option<&str>) -> Workflow {
        self.workflow(
            WorkflowTrigger::DocumentUploaded,
            WorkflowConditions::default(),
            vec![webhook(url, secret)],
        )
        .await
    }

    async fn tag(&self, name: &str) -> Id {
        self.tags
            .create(CreateTag {
                name: name.into(),
                color: None,
            })
            .await
            .unwrap()
            .id
    }

    async fn document(&self, filename: &str, mime: &str) -> Document {
        self.documents
            .create(CreateDocument {
                title: filename.into(),
                content: None,
                original_filename: Some(filename.into()),
                mime_type: Some(mime.into()),
                correspondent_id: None,
                document_type_id: None,
            })
            .await
            .unwrap()
    }

    /// Queue a run for `workflow` the way the engine would
    async fn run_for(&self, workflow: &Workflow, document_id: Id) -> WorkflowRun {
        let run = WorkflowRun::new(
            workflow.id,
            workflow.trigger,
            Some(document_id),
            r#"{"event":"document.uploaded","version":1}"#.into(),
        );
        assert!(self.workflows.create_run(&run).await.unwrap());
        run
    }

    async fn run(&self, id: Id) -> WorkflowRun {
        self.workflows.find_run(id).await.unwrap().unwrap()
    }
}

fn webhook(url: String, secret: Option<&str>) -> WorkflowAction {
    WorkflowAction::Webhook {
        url,
        method: WebhookMethod::Post,
        headers: BTreeMap::from([("X-Custom".to_string(), "yes".to_string())]),
        secret: secret.map(str::to_string),
    }
}

fn unreachable_hook() -> Vec<WorkflowAction> {
    vec![webhook("http://127.0.0.1:1/x".into(), None)]
}

fn conditions_for_mime(mime: &str) -> WorkflowConditions {
    WorkflowConditions {
        mime_type: Some(mime.into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn engine_only_queues_workflows_whose_conditions_match() {
    let h = Harness::new().await;
    let tag = h.tag("tax").await;
    let uploaded = WorkflowTrigger::DocumentUploaded;

    let pdf = h
        .workflow(
            uploaded,
            conditions_for_mime("application/pdf"),
            unreachable_hook(),
        )
        .await;
    let glob = h
        .workflow(
            uploaded,
            WorkflowConditions {
                filename_glob: Some("scan-*.PDF".into()),
                ..Default::default()
            },
            unreachable_hook(),
        )
        .await;
    h.workflow(uploaded, conditions_for_mime("image/*"), unreachable_hook())
        .await;
    h.workflow(
        uploaded,
        WorkflowConditions {
            tag_id: Some(tag),
            ..Default::default()
        },
        unreachable_hook(),
    )
    .await;
    h.workflow(
        WorkflowTrigger::DocumentDeleted,
        WorkflowConditions::default(),
        unreachable_hook(),
    )
    .await;
    let disabled = h
        .workflow(uploaded, WorkflowConditions::default(), unreachable_hook())
        .await;
    h.workflows
        .update(
            disabled.id,
            WorkflowInput {
                name: disabled.name,
                enabled: false,
                trigger: disabled.trigger,
                conditions: disabled.conditions,
                actions: disabled.actions,
            },
        )
        .await
        .unwrap();

    let doc = h.document("scan-001.pdf", "application/pdf").await;
    let runs = h
        .engine
        .handle(&DocumentEvent::Created {
            document_id: doc.id,
        })
        .await
        .unwrap();

    let mut matched: Vec<Id> = runs.iter().map(|r| r.workflow_id).collect();
    matched.sort();
    let mut expected = vec![pdf.id, glob.id];
    expected.sort();
    assert_eq!(matched, expected);
    assert_eq!(h.queue.pending_count().await.unwrap(), 2);
}

#[tokio::test]
async fn tag_conditions_are_evaluated_against_the_documents_tags() {
    let h = Harness::new().await;
    let tag = h.tag("tax").await;
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentUpdated,
            WorkflowConditions {
                tag_id: Some(tag),
                ..Default::default()
            },
            unreachable_hook(),
        )
        .await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let event = DocumentEvent::Updated {
        document_id: doc.id,
    };

    assert!(h.engine.handle(&event).await.unwrap().is_empty());
    h.documents.add_tag(doc.id, tag).await.unwrap();
    let runs = h.engine.handle(&event).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].workflow_id, workflow.id);
}

#[tokio::test]
async fn upload_creates_exactly_one_uploaded_run() {
    let h = Harness::new().await;
    let workflow = h
        .webhook_workflow("http://127.0.0.1:1/x".into(), None)
        .await;

    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(h.engine.clone().run(h.events.subscribe(), shutdown_rx));

    let doc = h.document("a.pdf", "application/pdf").await;
    // Bookkeeping writes after the upload must not fire anything.
    h.documents.set_file_path(doc.id, "a/a.pdf").await.unwrap();
    h.documents.set_page_count(doc.id, 3).await.unwrap();

    let mut runs = Vec::new();
    for _ in 0..100 {
        runs = h.workflows.list_runs(workflow.id, 10).await.unwrap();
        if !runs.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].event, WorkflowTrigger::DocumentUploaded);
    assert_eq!(runs[0].document_id, Some(doc.id));
    assert_eq!(runs[0].status, RunStatus::Pending);

    // A repeated event for the same document is ignored.
    let again = h
        .engine
        .handle(&DocumentEvent::Created {
            document_id: doc.id,
        })
        .await
        .unwrap();
    assert!(again.is_empty());
    assert_eq!(
        h.workflows.list_runs(workflow.id, 10).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn concurrent_events_still_create_one_run() {
    let h = Harness::new().await;
    let workflow = h
        .webhook_workflow("http://127.0.0.1:1/x".into(), None)
        .await;
    let doc = h.document("a.pdf", "application/pdf").await;

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let engine = h.engine.clone();
            tokio::spawn(async move {
                engine
                    .handle(&DocumentEvent::Created {
                        document_id: doc.id,
                    })
                    .await
                    .unwrap()
                    .len()
            })
        })
        .collect();
    let mut created = 0;
    for handle in handles {
        created += handle.await.unwrap();
    }

    assert_eq!(created, 1);
    assert_eq!(
        h.workflows.list_runs(workflow.id, 10).await.unwrap().len(),
        1
    );
    assert_eq!(h.queue.pending_count().await.unwrap(), 1);
}

#[tokio::test]
async fn processed_fires_once_per_document() {
    let h = Harness::new().await;
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentProcessed,
            WorkflowConditions::default(),
            unreachable_hook(),
        )
        .await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let event = DocumentEvent::Processed {
        document_id: doc.id,
    };

    assert_eq!(h.engine.handle(&event).await.unwrap().len(), 1);
    assert!(h.engine.handle(&event).await.unwrap().is_empty());
    assert_eq!(
        h.workflows.list_runs(workflow.id, 10).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn deleted_runs_carry_the_document_as_it_was() {
    let h = Harness::new().await;
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentDeleted,
            conditions_for_mime("application/pdf"),
            unreachable_hook(),
        )
        .await;
    let sender = h
        .correspondents
        .create(CreateCorrespondent {
            name: "ACME".into(),
        })
        .await
        .unwrap();
    let doc = h.document("gone.pdf", "application/pdf").await;
    h.raw_documents
        .update(
            doc.id,
            UpdateDocument {
                correspondent_id: Some(Some(sender.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let mut rx = h.events.subscribe();
    h.documents.delete(doc.id).await.unwrap();
    let event = loop {
        let event = rx.recv().await.unwrap();
        if matches!(event, DocumentEvent::Deleted { .. }) {
            break event;
        }
    };

    let runs = h.engine.handle(&event).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].workflow_id, workflow.id);
    let payload: serde_json::Value = serde_json::from_str(&runs[0].payload).unwrap();
    assert_eq!(payload["event"], "document.deleted");
    assert_eq!(payload["document"]["id"], doc.id.to_string());
    assert_eq!(payload["document"]["filename"], "gone.pdf");
    assert_eq!(payload["document"]["correspondent"]["name"], "ACME");
}

#[tokio::test]
async fn webhook_is_signed_over_timestamp_and_body() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    let workflow = h.webhook_workflow(server.url("hook"), Some("s3cret")).await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;

    assert_eq!(
        h.runner.execute(run.id).await.unwrap(),
        RunOutcome::Succeeded
    );

    {
        let received = server.state.received.lock().unwrap();
        assert_eq!(received.len(), 1);
        let request = &received[0];
        let header = |name: &str| {
            request
                .headers
                .get(name)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        };

        assert_eq!(request.body, run.payload);
        let timestamp: i64 = header("x-folio-timestamp").parse().unwrap();
        assert_eq!(
            header("x-folio-signature"),
            sign("s3cret", timestamp, &request.body)
        );
        assert!(header("x-folio-signature").starts_with("sha256="));
        assert_eq!(header("x-folio-event"), "document.uploaded");
        assert_eq!(header("x-folio-delivery"), run.id.to_string());
        assert!(header("user-agent").starts_with("Folio-Webhooks/"));
        assert_eq!(header("content-type"), "application/json");
        assert_eq!(header("x-custom"), "yes");
    }

    let done = h.run(run.id).await;
    assert_eq!(done.status, RunStatus::Succeeded);
    assert_eq!(done.response_status, Some(200));
    assert_eq!(done.attempts, 1);
    assert!(done.finished_at.is_some());
}

#[tokio::test]
async fn signature_matches_a_known_vector() {
    // Computed with Python's hmac module: HMAC-SHA256("key", "1700000000.{}")
    assert_eq!(
        sign("key", 1_700_000_000, "{}"),
        "sha256=9d713ed406bb7076d4123f0dc2c39d2df5c654ed4b0cd56b52c8b4c940bd63ae"
    );
}

#[tokio::test]
async fn unsigned_when_no_secret_is_set() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    let workflow = h.webhook_workflow(server.url("hook"), None).await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;

    h.runner.execute(run.id).await.unwrap();

    let received = server.state.received.lock().unwrap();
    assert!(received[0].headers.get("x-folio-signature").is_none());
}

#[tokio::test]
async fn retries_on_server_error_then_succeeds() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    server.script("hook", &[500]);
    let workflow = h.webhook_workflow(server.url("hook"), Some("k")).await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;

    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Retry);
    let after_first = h.run(run.id).await;
    assert_eq!(after_first.status, RunStatus::Pending);
    assert_eq!(after_first.attempts, 1);
    assert_eq!(after_first.response_status, Some(500));
    assert!(after_first.last_error.unwrap().contains("500"));

    assert_eq!(
        h.runner.execute(run.id).await.unwrap(),
        RunOutcome::Succeeded
    );
    let done = h.run(run.id).await;
    assert_eq!(done.status, RunStatus::Succeeded);
    assert_eq!(done.attempts, 2);
    assert_eq!(done.response_status, Some(200));
    assert_eq!(done.last_error, None);

    let received = server.state.received.lock().unwrap();
    assert_eq!(
        received[0].headers["x-folio-delivery"], received[1].headers["x-folio-delivery"],
        "retries reuse the delivery id"
    );
}

#[tokio::test]
async fn gives_up_after_max_attempts_and_records_the_error() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    server.script("hook", &[503; 10]);
    let workflow = h.webhook_workflow(server.url("hook"), None).await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;

    for attempt in 1..MAX_ATTEMPTS {
        assert_eq!(
            h.runner.execute(run.id).await.unwrap(),
            RunOutcome::Retry,
            "attempt {attempt}"
        );
    }
    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Failed);

    let failed = h.run(run.id).await;
    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.attempts, MAX_ATTEMPTS);
    assert_eq!(failed.response_status, Some(503));
    assert!(failed.last_error.unwrap().contains("503"));
    assert!(failed.finished_at.is_some());
    assert_eq!(server.hits("hook"), MAX_ATTEMPTS as usize);

    // A finished run is never executed again.
    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Failed);
    assert_eq!(server.hits("hook"), MAX_ATTEMPTS as usize);
}

#[tokio::test]
async fn client_errors_are_not_retried() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    server.script("hook", &[404]);
    let workflow = h.webhook_workflow(server.url("hook"), None).await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;

    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Failed);
    let failed = h.run(run.id).await;
    assert_eq!(failed.attempts, 1);
    assert_eq!(failed.response_status, Some(404));
}

#[tokio::test]
async fn unreachable_targets_are_retried() {
    let h = Harness::new().await;
    // Port 1 refuses connections.
    let workflow = h
        .webhook_workflow("http://127.0.0.1:1/hook".into(), None)
        .await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;

    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Retry);
    let pending = h.run(run.id).await;
    assert_eq!(pending.response_status, None);
    assert!(pending.last_error.is_some());
}

#[tokio::test]
async fn retry_resumes_after_the_actions_that_already_finished() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    server.script("second", &[500]);
    let tag = h.tag("sent").await;
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentUploaded,
            WorkflowConditions::default(),
            vec![
                webhook(server.url("first"), None),
                webhook(server.url("second"), None),
                WorkflowAction::AddTags { tag_ids: vec![tag] },
            ],
        )
        .await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;

    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Retry);
    assert_eq!(
        h.runner.execute(run.id).await.unwrap(),
        RunOutcome::Succeeded
    );

    assert_eq!(server.hits("first"), 1);
    assert_eq!(server.hits("second"), 2);
    assert_eq!(h.raw_documents.get_tags(doc.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn local_actions_change_the_document_without_emitting_events() {
    let h = Harness::new().await;
    let tag = h.tag("auto").await;
    let sender = h
        .correspondents
        .create(CreateCorrespondent {
            name: "ACME".into(),
        })
        .await
        .unwrap();
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentUploaded,
            WorkflowConditions::default(),
            vec![
                WorkflowAction::AddTags { tag_ids: vec![tag] },
                WorkflowAction::SetCorrespondent {
                    correspondent_id: sender.id,
                },
            ],
        )
        .await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;

    let mut rx = h.events.subscribe();
    assert_eq!(
        h.runner.execute(run.id).await.unwrap(),
        RunOutcome::Succeeded
    );

    let updated = h.raw_documents.find_by_id(doc.id).await.unwrap().unwrap();
    assert_eq!(updated.correspondent_id, Some(sender.id));
    assert_eq!(h.raw_documents.get_tags(doc.id).await.unwrap()[0].id, tag);
    assert!(
        rx.try_recv().is_err(),
        "workflow edits must not re-trigger workflows"
    );
}

#[tokio::test]
async fn client_refuses_non_http_urls() {
    let client = WebhookClient::new();
    let headers = BTreeMap::new();
    for url in ["file:///etc/passwd", "ftp://example.com/x"] {
        let result = client
            .send(WebhookRequest {
                url,
                headers: &headers,
                secret: None,
                event: WorkflowTrigger::DocumentUploaded,
                delivery_id: Uuid::new_v4(),
                body: "{}",
            })
            .await;
        assert!(!result.is_success(), "{url}");
        assert!(!result.retryable, "{url}");
        assert_eq!(result.status, None);
    }
}

#[tokio::test]
async fn deleting_a_workflow_removes_its_runs() {
    let h = Harness::new().await;
    let workflow = h
        .webhook_workflow("http://127.0.0.1:1/x".into(), None)
        .await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;

    h.workflows.delete(workflow.id).await.unwrap();
    assert!(h.workflows.find_run(run.id).await.unwrap().is_none());
    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Failed);
}
