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
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, OutboxStore,
    TagRepository, VaultRepository, WorkflowRepository,
};
use folio_core::services::{
    BroadcastConsumer, DispatcherConfig, EventConsumer, OutboxDispatcher, WorkflowEngine,
};
use folio_core::{DocumentEvent, EventBroadcaster, EventOrigin, OutboxEvent};
use folio_infra::workflow::{sign, RunOutcome, WebhookRequest, MAX_ATTEMPTS};
use folio_infra::{
    Database, JobQueue, ObjectStoreDocumentStore, StorageFileMetadata, WebhookClient,
    WorkflowRunner,
};

struct Received {
    path: String,
    headers: HeaderMap,
    body: String,
}

#[derive(Default)]
struct ServerState {
    received: Mutex<Vec<Received>>,
    effects: Mutex<HashMap<String, String>>,
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
            if let (Some(delivery), Some(route)) =
                (headers.get("x-folio-delivery"), headers.get("x-route"))
            {
                state
                    .effects
                    .lock()
                    .unwrap()
                    .entry(delivery.to_str().unwrap().to_string())
                    .or_insert_with(|| route.to_str().unwrap().to_string());
            }
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
    vault: folio_core::models::VaultId,
    workflows: Arc<dyn WorkflowRepository>,
    documents: Arc<dyn DocumentRepository>,
    tags: Arc<dyn TagRepository>,
    document_types: Arc<dyn DocumentTypeRepository>,
    vaults: Arc<dyn VaultRepository>,
    correspondents: Arc<dyn CorrespondentRepository>,
    queue: JobQueue,
    outbox: Arc<dyn OutboxStore>,
    pool: sqlx::SqlitePool,
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
        let database = Database::connect(&format!("sqlite:{}/test.db?mode=rwc", dir.display()))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let repos = database.repositories().await;
        let vault = repos.vaults.instance_default().await.unwrap();

        let Database::Sqlite(pool) = database else {
            unreachable!("connected to SQLite");
        };
        let workflows = repos.workflows;
        let documents = repos.documents;
        let tags = repos.tags;
        let correspondents = repos.correspondents;
        let queue = repos.jobs;

        let engine = Arc::new(WorkflowEngine::new(
            workflows.clone(),
            documents.clone(),
            correspondents.clone(),
            repos.document_types.clone(),
            Arc::new(StorageFileMetadata::new(Arc::new(
                ObjectStoreDocumentStore::in_memory(),
            ))),
            "http://folio.test",
        ));
        let runner = WorkflowRunner::new(
            workflows.clone(),
            repos.workflow_documents,
            WebhookClient::new(),
        );

        Self {
            vault,
            workflows,
            documents,
            tags,
            document_types: repos.document_types,
            vaults: repos.vaults,
            correspondents,
            queue,
            outbox: repos.outbox,
            pool,
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
            .create(
                self.vault,
                WorkflowInput {
                    name: "test".into(),
                    enabled: true,
                    trigger,
                    conditions,
                    actions,
                },
            )
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
            .create(
                self.vault,
                CreateTag {
                    name: name.into(),
                    color: None,
                },
            )
            .await
            .unwrap()
            .id
    }

    async fn document(&self, filename: &str, mime: &str) -> Document {
        self.documents
            .create(
                self.vault,
                CreateDocument {
                    title: filename.into(),
                    content: None,
                    original_filename: Some(filename.into()),
                    mime_type: Some(mime.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
    }

    /// Queue a run for `workflow` the way the engine would
    async fn run_for(&self, workflow: &Workflow, document_id: Id) -> WorkflowRun {
        let run = WorkflowRun::new(
            workflow.vault_id,
            workflow.id,
            workflow.trigger,
            Some(document_id),
            r#"{"event":"document.uploaded","version":1}"#.into(),
        )
        .with_actions(workflow.actions.clone());
        assert!(self.workflows.create_run(&run).await.unwrap());
        run
    }

    async fn run(&self, id: Id) -> WorkflowRun {
        self.workflows.find_run(id).await.unwrap().unwrap()
    }

    async fn event(&self, event: DocumentEvent) -> OutboxEvent {
        let id = self
            .outbox
            .append_document(self.vault, &event, EventOrigin::User)
            .await
            .unwrap();
        self.outbox
            .replay(id - 1, chrono::Utc::now() - chrono::Duration::days(1), 1)
            .await
            .unwrap()
            .pop()
            .unwrap()
    }

    async fn handle(&self, event: DocumentEvent) -> Vec<WorkflowRun> {
        self.engine.handle(&self.event(event).await).await.unwrap()
    }

    fn dispatcher(&self, consumers: Vec<Arc<dyn EventConsumer>>) -> Arc<OutboxDispatcher> {
        Arc::new(OutboxDispatcher::new(
            self.outbox.clone(),
            consumers,
            DispatcherConfig {
                poll_interval: Duration::from_millis(20),
                ..Default::default()
            },
        ))
    }

    /// Everything not yet claimed, as a dispatcher would see it
    async fn pending_events(&self) -> Vec<OutboxEvent> {
        self.outbox
            .claim(1000, chrono::Duration::seconds(30), 10)
            .await
            .unwrap()
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
            h.vault,
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
        .handle(DocumentEvent::Created {
            document_id: doc.id,
        })
        .await;

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

    assert!(h.handle(event.clone()).await.is_empty());
    h.documents.add_tag(h.vault, doc.id, tag).await.unwrap();
    let runs = h.handle(event).await;
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
    tokio::spawn(h.dispatcher(vec![h.engine.clone()]).run(shutdown_rx));

    let doc = h.document("a.pdf", "application/pdf").await;
    // Bookkeeping writes after the upload must not fire anything.
    h.documents
        .set_file_path(h.vault, doc.id, "a/a.pdf")
        .await
        .unwrap();
    h.documents
        .set_page_count(h.vault, doc.id, 3)
        .await
        .unwrap();

    let mut runs = Vec::new();
    for _ in 0..100 {
        runs = h
            .workflows
            .list_runs(h.vault, workflow.id, 10)
            .await
            .unwrap();
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
        .handle(DocumentEvent::Created {
            document_id: doc.id,
        })
        .await;
    assert!(again.is_empty());
    assert_eq!(
        h.workflows
            .list_runs(h.vault, workflow.id, 10)
            .await
            .unwrap()
            .len(),
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

    let mut handles = Vec::new();
    for _ in 0..8 {
        let engine = h.engine.clone();
        let event = h
            .event(DocumentEvent::Created {
                document_id: doc.id,
            })
            .await;
        handles.push(tokio::spawn(async move {
            engine.handle(&event).await.unwrap().len()
        }));
    }
    let mut created = 0;
    for handle in handles {
        created += handle.await.unwrap();
    }

    assert_eq!(created, 1);
    assert_eq!(
        h.workflows
            .list_runs(h.vault, workflow.id, 10)
            .await
            .unwrap()
            .len(),
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

    assert_eq!(h.handle(event.clone()).await.len(), 1);
    assert!(h.handle(event).await.is_empty());
    assert_eq!(
        h.workflows
            .list_runs(h.vault, workflow.id, 10)
            .await
            .unwrap()
            .len(),
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
        .create(
            h.vault,
            CreateCorrespondent {
                name: "ACME".into(),
            },
        )
        .await
        .unwrap();
    let doc = h.document("gone.pdf", "application/pdf").await;
    h.documents
        .update(
            h.vault,
            doc.id,
            UpdateDocument {
                correspondent_id: Some(Some(sender.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    h.documents.delete(h.vault, doc.id).await.unwrap();
    let trash_event = h
        .pending_events()
        .await
        .into_iter()
        .find(|e| matches!(e.event, Some(DocumentEvent::Trashed { .. })))
        .unwrap();
    assert!(h.engine.handle(&trash_event).await.unwrap().is_empty());
    h.documents.request_purge(h.vault, doc.id).await.unwrap();
    h.documents.finalize_purge(h.vault, doc.id).await.unwrap();
    let event = h
        .pending_events()
        .await
        .into_iter()
        .find(|e| matches!(e.event, Some(DocumentEvent::Deleted { .. })))
        .expect("the delete left an event in the outbox");

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
async fn queued_workflow_does_not_send_a_trashed_document() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    let workflow = h.webhook_workflow(server.url("trashed"), None).await;
    let document = h.document("private.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, document.id).await;
    h.documents.delete(h.vault, document.id).await.unwrap();

    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Failed);

    assert_eq!(server.hits("trashed"), 0);
    assert!(h
        .workflows
        .find_run(run.id)
        .await
        .unwrap()
        .unwrap()
        .finished_at
        .is_some());
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
        assert!(Uuid::parse_str(&header("x-folio-delivery")).is_ok());
        assert_eq!(header("x-folio-run"), run.id.to_string());
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
    assert_eq!(
        h.documents.get_tags(h.vault, doc.id).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn local_actions_are_announced_but_never_trigger_workflows() {
    let h = Harness::new().await;
    let tag = h.tag("auto").await;
    let sender = h
        .correspondents
        .create(
            h.vault,
            CreateCorrespondent {
                name: "ACME".into(),
            },
        )
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
    let on_update = h
        .workflow(
            WorkflowTrigger::DocumentUpdated,
            WorkflowConditions::default(),
            unreachable_hook(),
        )
        .await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;
    // The upload's own event is not under test
    h.pending_events().await;

    assert_eq!(
        h.runner.execute(run.id).await.unwrap(),
        RunOutcome::Succeeded
    );

    let updated = h
        .documents
        .find_by_id(h.vault, doc.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.correspondent_id, Some(sender.id));
    assert_eq!(
        h.documents.get_tags(h.vault, doc.id).await.unwrap()[0].id,
        tag
    );

    let events = h.pending_events().await;
    let projections: Vec<_> = events
        .iter()
        .filter(|event| event.event.is_some())
        .collect();
    assert_eq!(
        projections.len(),
        2,
        "the tag edit and the correspondent change"
    );
    assert!(events
        .iter()
        .all(|event| event.origin == EventOrigin::Workflow));
    assert!(projections.iter().all(|event| matches!(event.event, Some(DocumentEvent::Updated { document_id }) if document_id == doc.id)));

    // Dispatching them reaches live clients but starts no workflow.
    let broadcaster = EventBroadcaster::default();
    let mut live = broadcaster.subscribe();
    for event in &events {
        h.engine.consume(event).await.unwrap();
        BroadcastConsumer(broadcaster.clone())
            .consume(event)
            .await
            .unwrap();
    }
    assert!(h
        .workflows
        .list_runs(h.vault, on_update.id, 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(live.try_recv().unwrap().id, events[0].id);
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
                run_id: Uuid::new_v4(),
                action_index: 0,
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

    h.workflows.delete(h.vault, workflow.id).await.unwrap();
    assert!(h.workflows.find_run(run.id).await.unwrap().is_none());
    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Failed);
}

#[tokio::test]
async fn a_lagging_broadcast_does_not_lose_runs() {
    let h = Harness::new().await;
    let workflow = h
        .webhook_workflow("http://127.0.0.1:1/x".into(), None)
        .await;
    // A subscriber that never reads overflows the tiny buffer many times over.
    let broadcaster = EventBroadcaster::new(2);
    let _idle = broadcaster.subscribe();
    for i in 0..30 {
        h.document(&format!("{i}.pdf"), "application/pdf").await;
    }

    let dispatcher = h.dispatcher(vec![
        h.engine.clone(),
        Arc::new(BroadcastConsumer(broadcaster)),
    ]);
    while dispatcher.dispatch_once().await.unwrap() > 0 {}

    assert_eq!(
        h.workflows
            .list_runs(h.vault, workflow.id, 100)
            .await
            .unwrap()
            .len(),
        30
    );
    assert_eq!(h.queue.pending_count().await.unwrap(), 30);
}

#[tokio::test]
async fn handling_an_event_twice_creates_one_run() {
    let h = Harness::new().await;
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentUpdated,
            WorkflowConditions::default(),
            unreachable_hook(),
        )
        .await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let event = h
        .event(DocumentEvent::Updated {
            document_id: doc.id,
        })
        .await;

    assert_eq!(h.engine.handle(&event).await.unwrap().len(), 1);
    assert!(h.engine.handle(&event).await.unwrap().is_empty());
    assert_eq!(
        h.workflows
            .list_runs(h.vault, workflow.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(h.queue.pending_count().await.unwrap(), 1);

    // A later, distinct update is a new occurrence and runs again.
    assert_eq!(
        h.handle(DocumentEvent::Updated {
            document_id: doc.id
        })
        .await
        .len(),
        1
    );
}

#[tokio::test]
async fn source_vault_envelope_cannot_trigger_destination_workflows_after_a_move() {
    use folio_core::{DomainEventEnvelope, DomainEventSubject, EventAudience};
    let h = Harness::new().await;
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentUpdated,
            WorkflowConditions::default(),
            unreachable_hook(),
        )
        .await;
    let repos = Database::from(h.pool.clone()).repositories().await;
    let source = repos
        .vaults
        .create("Original vault", None)
        .await
        .unwrap()
        .id;
    let doc = repos
        .documents
        .create(
            source,
            CreateDocument {
                title: "Moved".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let envelope = DomainEventEnvelope::new(
        "document.updated",
        source,
        DomainEventSubject {
            subject_type: "document".into(),
            id: doc.id,
        },
        Some(doc.id),
        EventAudience::Vault,
        serde_json::json!({}),
    )
    .unwrap();
    repos
        .documents
        .move_to(doc.id, source, h.vault, None)
        .await
        .unwrap();
    let mut event = h
        .event(DocumentEvent::Updated {
            document_id: doc.id,
        })
        .await;
    event.envelope = Some(envelope);

    assert!(h.engine.handle(&event).await.unwrap().is_empty());
    assert!(h
        .workflows
        .list_runs(h.vault, workflow.id, 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(h.queue.pending_count().await.unwrap(), 0);
}

#[tokio::test]
async fn user_private_envelope_cannot_trigger_ordinary_document_workflows() {
    use folio_core::{DomainEventEnvelope, DomainEventSubject, EventAudience};
    let h = Harness::new().await;
    h.workflow(
        WorkflowTrigger::DocumentUpdated,
        WorkflowConditions::default(),
        unreachable_hook(),
    )
    .await;
    let doc = h.document("private.pdf", "application/pdf").await;
    let mut event = h
        .event(DocumentEvent::Updated {
            document_id: doc.id,
        })
        .await;
    event.envelope = Some(
        DomainEventEnvelope::new(
            "document.updated",
            h.vault,
            DomainEventSubject {
                subject_type: "document".into(),
                id: doc.id,
            },
            Some(doc.id),
            EventAudience::User {
                user_id: folio_core::models::UserId::from("private-owner"),
            },
            serde_json::json!({}),
        )
        .unwrap(),
    );

    assert!(h.engine.handle(&event).await.unwrap().is_empty());
    assert_eq!(h.queue.pending_count().await.unwrap(), 0);
}

#[tokio::test]
async fn one_failing_workflow_does_not_skip_the_others() {
    let h = Harness::new().await;
    let broken = h
        .webhook_workflow("http://127.0.0.1:1/a".into(), None)
        .await;
    let healthy = h
        .webhook_workflow("http://127.0.0.1:1/b".into(), None)
        .await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let event = h
        .event(DocumentEvent::Created {
            document_id: doc.id,
        })
        .await;

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TRIGGER refuse BEFORE INSERT ON workflow_runs
         WHEN NEW.workflow_id = '{}' BEGIN SELECT RAISE(ABORT, 'refused'); END",
        broken.id
    )))
    .execute(&h.pool)
    .await
    .unwrap();
    assert!(h.engine.handle(&event).await.is_err());
    assert_eq!(
        h.workflows
            .list_runs(h.vault, healthy.id, 10)
            .await
            .unwrap()
            .len(),
        1,
        "the healthy workflow ran despite the failure before it"
    );

    // The dispatcher retries the event: the missing run appears, none is duplicated.
    sqlx::query("DROP TRIGGER refuse")
        .execute(&h.pool)
        .await
        .unwrap();
    let retried = h.engine.handle(&event).await.unwrap();
    assert_eq!(retried.len(), 1);
    assert_eq!(retried[0].workflow_id, broken.id);
    assert_eq!(h.queue.pending_count().await.unwrap(), 2);
}

#[tokio::test]
async fn editing_a_workflow_does_not_change_queued_runs() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    let workflow = h.webhook_workflow(server.url("before"), None).await;
    let doc = h.document("a.pdf", "application/pdf").await;
    let runs = h
        .handle(DocumentEvent::Created {
            document_id: doc.id,
        })
        .await;

    h.workflows
        .update(
            h.vault,
            workflow.id,
            WorkflowInput {
                name: workflow.name,
                enabled: true,
                trigger: workflow.trigger,
                conditions: workflow.conditions,
                actions: vec![webhook(server.url("after"), None)],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        h.runner.execute(runs[0].id).await.unwrap(),
        RunOutcome::Succeeded
    );

    assert_eq!(server.hits("before"), 1);
    assert_eq!(server.hits("after"), 0);
}

#[tokio::test]
async fn webhook_actions_have_independent_ids_and_retries_deduplicate() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    server.script("hook", &[200, 503]);
    let actions = ["alpha", "beta"]
        .into_iter()
        .map(|route| WorkflowAction::Webhook {
            url: server.url("hook"),
            method: WebhookMethod::Post,
            headers: BTreeMap::from([
                ("X-Route".into(), route.into()),
                ("X-Folio-Run".into(), "spoofed".into()),
                ("X-Folio-Delivery".into(), "spoofed".into()),
            ]),
            secret: None,
        })
        .collect();
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentUploaded,
            WorkflowConditions::default(),
            actions,
        )
        .await;
    let document = h.document("invoice.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, document.id).await;
    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Retry);
    assert_eq!(
        h.runner.execute(run.id).await.unwrap(),
        RunOutcome::Succeeded
    );
    let mut effects: Vec<_> = server
        .state
        .effects
        .lock()
        .unwrap()
        .values()
        .cloned()
        .collect();
    effects.sort();
    assert_eq!(effects, vec!["alpha", "beta"]);
    let received = server.state.received.lock().unwrap();
    assert_eq!(received.len(), 3);
    for request in received.iter() {
        assert_eq!(request.headers["x-folio-run"], run.id.to_string());
    }
    assert_ne!(
        received[0].headers["x-folio-delivery"],
        received[1].headers["x-folio-delivery"]
    );
    assert_eq!(
        received[1].headers["x-folio-delivery"],
        received[2].headers["x-folio-delivery"]
    );
    assert_eq!(
        received
            .iter()
            .map(|r| r.headers["x-route"].to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta", "beta"]
    );
}

#[tokio::test]
async fn delayed_events_match_and_deliver_their_own_metadata_names_and_occurrence_time() {
    use folio_core::models::{
        CreateDocumentType, UpdateCorrespondent, UpdateDocumentType, UpdateTag,
    };
    let h = Harness::new().await;
    let first_tag = h.tag("first tag").await;
    let later_tag = h.tag("later tag").await;
    let sender = h
        .correspondents
        .create(
            h.vault,
            CreateCorrespondent {
                name: "first sender".into(),
            },
        )
        .await
        .unwrap();
    let kind = h
        .document_types
        .create(
            h.vault,
            CreateDocumentType {
                name: "first type".into(),
            },
        )
        .await
        .unwrap();
    let first_workflow = h
        .workflow(
            WorkflowTrigger::DocumentUpdated,
            WorkflowConditions {
                tag_id: Some(first_tag),
                correspondent_id: Some(sender.id),
                document_type_id: Some(kind.id),
                ..Default::default()
            },
            unreachable_hook(),
        )
        .await;
    let later_workflow = h
        .workflow(
            WorkflowTrigger::DocumentUpdated,
            WorkflowConditions {
                tag_id: Some(later_tag),
                ..Default::default()
            },
            unreachable_hook(),
        )
        .await;
    let doc = h
        .documents
        .create(
            h.vault,
            CreateDocument {
                title: "first title".into(),
                original_filename: Some("first.pdf".into()),
                mime_type: Some("application/pdf".into()),
                correspondent_id: Some(sender.id),
                document_type_id: Some(kind.id),
                tag_ids: vec![first_tag],
                source: folio_core::models::DocumentSource::folder(
                    "original source",
                    folio_core::models::FolderOrigin {
                        relative_path: "original/first.pdf".into(),
                        original_filename: "first.pdf".into(),
                    },
                ),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut first = h
        .event(DocumentEvent::Updated {
            document_id: doc.id,
        })
        .await;
    first.created_at = chrono::DateTime::from_timestamp(1_600_000_000, 0).unwrap();
    h.correspondents
        .update(
            h.vault,
            sender.id,
            UpdateCorrespondent {
                name: Some("later sender".into()),
            },
        )
        .await
        .unwrap();
    h.document_types
        .update(
            h.vault,
            kind.id,
            UpdateDocumentType {
                name: Some("later type".into()),
            },
        )
        .await
        .unwrap();
    h.documents
        .update(
            h.vault,
            doc.id,
            UpdateDocument {
                title: Some("later title".into()),
                tag_ids: Some(vec![later_tag]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let later = h
        .event(DocumentEvent::Updated {
            document_id: doc.id,
        })
        .await;
    h.correspondents
        .update(
            h.vault,
            sender.id,
            UpdateCorrespondent {
                name: Some("current sender".into()),
            },
        )
        .await
        .unwrap();
    h.document_types
        .update(
            h.vault,
            kind.id,
            UpdateDocumentType {
                name: Some("current type".into()),
            },
        )
        .await
        .unwrap();
    h.tags
        .update(
            h.vault,
            later_tag,
            UpdateTag {
                name: Some("current tag".into()),
                color: None,
            },
        )
        .await
        .unwrap();
    h.documents
        .update(
            h.vault,
            doc.id,
            UpdateDocument {
                title: Some("current title".into()),
                tag_ids: Some(Vec::new()),
                correspondent_id: Some(None),
                document_type_id: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let first_runs = h.engine.handle(&first).await.unwrap();
    let later_runs = h.engine.handle(&later).await.unwrap();

    assert_eq!(first_runs.len(), 1);
    assert_eq!(first_runs[0].workflow_id, first_workflow.id);
    assert_eq!(later_runs.len(), 1);
    assert_eq!(later_runs[0].workflow_id, later_workflow.id);
    let payload: serde_json::Value = serde_json::from_str(&first_runs[0].payload).unwrap();
    assert_eq!(payload["occurred_at"], "2020-09-13T12:26:40Z");
    assert_eq!(payload["document"]["title"], "first title");
    assert_eq!(payload["document"]["filename"], "first.pdf");
    assert_eq!(payload["document"]["correspondent"]["name"], "first sender");
    assert_eq!(payload["document"]["document_type"]["name"], "first type");
    assert_eq!(payload["document"]["tags"][0]["name"], "first tag");
    assert_eq!(
        payload["document"]["source"]["source_name"],
        "original source"
    );
    let payload: serde_json::Value = serde_json::from_str(&later_runs[0].payload).unwrap();
    assert_eq!(payload["document"]["title"], "later title");
    assert_eq!(payload["document"]["correspondent"]["name"], "later sender");
    assert_eq!(payload["document"]["document_type"]["name"], "later type");
    assert_eq!(payload["document"]["tags"][0]["name"], "later tag");
    assert!(h.engine.handle(&first).await.unwrap().is_empty());
    assert!(h.engine.handle(&later).await.unwrap().is_empty());
}

#[tokio::test]
async fn moving_before_dispatch_never_routes_an_event_to_destination_workflows() {
    let h = Harness::new().await;
    let origin = h
        .workflow(
            WorkflowTrigger::DocumentUpdated,
            Default::default(),
            unreachable_hook(),
        )
        .await;
    let destination = h.vaults.create("destination", None).await.unwrap().id;
    let foreign = h
        .workflows
        .create(
            destination,
            WorkflowInput {
                name: "destination hook".into(),
                enabled: true,
                trigger: WorkflowTrigger::DocumentUpdated,
                conditions: Default::default(),
                actions: unreachable_hook(),
            },
        )
        .await
        .unwrap();
    let doc = h.document("before move.pdf", "application/pdf").await;
    let event = h
        .event(DocumentEvent::Updated {
            document_id: doc.id,
        })
        .await;
    h.documents
        .move_to(doc.id, h.vault, destination, None)
        .await
        .unwrap();

    assert!(h.engine.handle(&event).await.unwrap().is_empty());

    assert!(h
        .workflows
        .list_runs(h.vault, origin.id, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(h
        .workflows
        .list_runs(destination, foreign.id, 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn moving_after_run_creation_terminally_fails_before_webhook_delivery() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    h.webhook_workflow(server.url("moved"), None).await;
    let doc = h.document("moved.pdf", "application/pdf").await;
    let run = h
        .handle(DocumentEvent::Created {
            document_id: doc.id,
        })
        .await
        .pop()
        .unwrap();
    let destination = h.vaults.create("destination", None).await.unwrap().id;
    h.documents
        .move_to(doc.id, h.vault, destination, None)
        .await
        .unwrap();

    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Failed);

    let failed = h.run(run.id).await;
    assert_eq!(failed.status, RunStatus::Failed);
    assert_eq!(failed.completed_actions, 0);
    assert!(failed.last_error.unwrap().contains("workflow's vault"));
    assert_eq!(server.hits("moved"), 0);
    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Failed);
}

#[tokio::test]
async fn moving_between_actions_stops_remaining_webhooks_and_local_changes() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    let doc = h.document("move on reply.pdf", "application/pdf").await;
    let destination = h.vaults.create("destination", None).await.unwrap().id;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let move_url = format!("http://{}/move", listener.local_addr().unwrap());
    let documents = h.documents.clone();
    let from = h.vault;
    let app = Router::new().route(
        "/move",
        post(move || {
            let documents = documents.clone();
            async move {
                documents
                    .move_to(doc.id, from, destination, None)
                    .await
                    .unwrap();
                StatusCode::OK
            }
        }),
    );
    let mover = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tag = h.tag("must not add").await;
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentUploaded,
            Default::default(),
            vec![
                webhook(move_url, None),
                webhook(server.url("remaining"), None),
                WorkflowAction::AddTags { tag_ids: vec![tag] },
            ],
        )
        .await;
    let run = h.run_for(&workflow, doc.id).await;

    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Failed);

    let failed = h.run(run.id).await;
    assert_eq!(failed.completed_actions, 1);
    assert_eq!(server.hits("remaining"), 0);
    assert!(h
        .documents
        .get_tags(destination, doc.id)
        .await
        .unwrap()
        .is_empty());
    assert!(failed.last_error.unwrap().contains("workflow's vault"));
    mover.abort();
}

#[tokio::test]
async fn moving_before_retry_preserves_completed_actions_and_stops_the_failed_action() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    server.script("retry", &[503]);
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentUploaded,
            Default::default(),
            vec![
                webhook(server.url("done"), None),
                webhook(server.url("retry"), None),
            ],
        )
        .await;
    let doc = h.document("retry.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;
    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Retry);
    let destination = h.vaults.create("destination", None).await.unwrap().id;
    h.documents
        .move_to(doc.id, h.vault, destination, None)
        .await
        .unwrap();

    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Failed);

    assert_eq!(h.run(run.id).await.completed_actions, 1);
    assert_eq!(server.hits("done"), 1);
    assert_eq!(server.hits("retry"), 1);
}

#[tokio::test]
async fn legacy_nondeletion_event_fails_with_context_error_and_exhausts_delivery_attempts() {
    let h = Harness::new().await;
    h.workflow(
        WorkflowTrigger::DocumentUpdated,
        Default::default(),
        unreachable_hook(),
    )
    .await;
    let doc = h.document("legacy.pdf", "application/pdf").await;
    let creation = h.pending_events().await.pop().unwrap();
    h.outbox.mark_dispatched(creation.id).await.unwrap();
    let payload = folio_core::events::encode_outbox_payload(
        &DocumentEvent::Updated {
            document_id: doc.id,
        },
        EventOrigin::User,
    );
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO outbox_events (type, payload, created_at, available_at) VALUES ('updated', ?, ?, ?) RETURNING id",
    ).bind(payload).bind(&now).bind(&now).fetch_one(&h.pool).await.unwrap();
    let dispatch = OutboxDispatcher::new(
        h.outbox.clone(),
        vec![h.engine.clone()],
        DispatcherConfig {
            max_attempts: 1,
            ..Default::default()
        },
    );

    assert_eq!(dispatch.dispatch_once().await.unwrap(), 1);

    let (attempts, dispatched, error): (i64, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT attempts, dispatched_at, last_error FROM outbox_events WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&h.pool)
    .await
    .unwrap();
    assert_eq!(attempts, 1);
    assert!(dispatched.is_none());
    let error = error.unwrap();
    assert!(error.contains(&id.to_string()));
    assert!(error.contains("historical document/vault context"));
    assert_eq!(dispatch.dispatch_once().await.unwrap(), 0);
    assert_eq!(h.queue.pending_count().await.unwrap(), 0);
}

#[tokio::test]
async fn legacy_deletion_snapshot_still_delivers_its_recorded_document_and_tags() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    let tag = h.tag("deleted tag").await;
    h.workflow(
        WorkflowTrigger::DocumentDeleted,
        WorkflowConditions {
            tag_id: Some(tag),
            ..Default::default()
        },
        vec![webhook(server.url("legacy-deleted"), None)],
    )
    .await;
    let doc = h.document("legacy gone.pdf", "application/pdf").await;
    h.documents.add_tag(h.vault, doc.id, tag).await.unwrap();
    h.documents.delete(h.vault, doc.id).await.unwrap();
    h.documents.request_purge(h.vault, doc.id).await.unwrap();
    h.documents.finalize_purge(h.vault, doc.id).await.unwrap();
    let mut event = h
        .pending_events()
        .await
        .into_iter()
        .find(|event| matches!(event.event, Some(DocumentEvent::Deleted { .. })))
        .unwrap();
    event.context = None;
    event.envelope = None;

    let runs = h.engine.handle(&event).await.unwrap();

    assert_eq!(runs.len(), 1);
    assert_eq!(
        h.runner.execute(runs[0].id).await.unwrap(),
        RunOutcome::Succeeded
    );
    let payload: serde_json::Value = serde_json::from_str(&runs[0].payload).unwrap();
    assert_eq!(payload["document"]["filename"], "legacy gone.pdf");
    assert_eq!(payload["document"]["tags"][0]["name"], "deleted tag");
    assert_eq!(server.hits("legacy-deleted"), 1);
}

#[tokio::test]
async fn missing_document_terminally_fails_before_nondeletion_webhook_delivery() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    let workflow = h.webhook_workflow(server.url("missing"), None).await;
    let doc = h.document("missing.pdf", "application/pdf").await;
    let run = h.run_for(&workflow, doc.id).await;
    h.documents.delete(h.vault, doc.id).await.unwrap();
    h.documents.request_purge(h.vault, doc.id).await.unwrap();
    h.documents.finalize_purge(h.vault, doc.id).await.unwrap();

    assert_eq!(h.runner.execute(run.id).await.unwrap(), RunOutcome::Failed);

    assert_eq!(server.hits("missing"), 0);
    assert_eq!(h.run(run.id).await.completed_actions, 0);
}

#[tokio::test]
async fn captured_deletion_context_takes_precedence_over_the_legacy_fallback() {
    let h = Harness::new().await;
    h.workflow(
        WorkflowTrigger::DocumentDeleted,
        Default::default(),
        unreachable_hook(),
    )
    .await;
    let doc = h.document("authoritative.pdf", "application/pdf").await;
    h.documents.delete(h.vault, doc.id).await.unwrap();
    h.documents.request_purge(h.vault, doc.id).await.unwrap();
    h.documents.finalize_purge(h.vault, doc.id).await.unwrap();
    let mut event = h
        .pending_events()
        .await
        .into_iter()
        .find(|event| matches!(event.event, Some(DocumentEvent::Deleted { .. })))
        .unwrap();
    let foreign = h.vaults.create("legacy fallback", None).await.unwrap().id;
    let DocumentEvent::Deleted {
        snapshot: Some(ref mut fallback),
        ..
    } = event.event.as_mut().unwrap()
    else {
        panic!("legacy fallback exists")
    };
    let fallback = Arc::make_mut(fallback);
    fallback.document.vault_id = foreign;
    fallback.document.title = "untrusted legacy title".into();

    let runs = h.engine.handle(&event).await.unwrap();

    assert_eq!(runs.len(), 1);
    let payload: serde_json::Value = serde_json::from_str(&runs[0].payload).unwrap();
    assert_eq!(payload["document"]["title"], "authoritative.pdf");
}

#[tokio::test]
async fn webhook_test_run_without_a_document_still_sends() {
    let h = Harness::new().await;
    let server = TestServer::start().await;
    let workflow = h.webhook_workflow(server.url("sample"), None).await;
    let run = WorkflowRun::new(h.vault, workflow.id, workflow.trigger, None, "{}".into())
        .with_actions(workflow.actions.clone());
    assert!(h.workflows.create_run(&run).await.unwrap());

    assert_eq!(
        h.runner.execute(run.id).await.unwrap(),
        RunOutcome::Succeeded
    );

    assert_eq!(server.hits("sample"), 1);
}

#[tokio::test]
async fn precise_metadata_dispatch_preserves_snapshot_identity_and_action_lineage() {
    use folio_core::models::UserId;
    use folio_core::mutation::with_actor;

    let h = Harness::new().await;
    let original_tag = h.tag("original condition").await;
    let action_tag = h.tag("workflow action").await;
    let doc = h.document("original.pdf", "application/pdf").await;
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentTagsChanged,
            WorkflowConditions {
                tag_id: Some(original_tag),
                ..Default::default()
            },
            vec![WorkflowAction::AddTags {
                tag_ids: vec![action_tag],
            }],
        )
        .await;
    h.documents
        .update(
            h.vault,
            doc.id,
            UpdateDocument {
                tag_ids: Some(vec![original_tag]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let source = h
        .pending_events()
        .await
        .into_iter()
        .find(|event| {
            event
                .envelope
                .as_ref()
                .is_some_and(|envelope| envelope.event_type() == "document.tags_changed")
        })
        .unwrap();
    assert!(source.event.is_none());
    let envelope = source.envelope.as_ref().unwrap();
    h.documents
        .update(
            h.vault,
            doc.id,
            UpdateDocument {
                title: Some("later title".into()),
                tag_ids: Some(vec![]),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let runs = h.engine.handle(&source).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].workflow_id, workflow.id);
    assert!(h.engine.handle(&source).await.unwrap().is_empty());
    let stored = h.run(runs[0].id).await;
    let payload: serde_json::Value = serde_json::from_str(&stored.payload).unwrap();
    assert_eq!(payload["document"]["title"], "original.pdf");
    assert_eq!(
        payload["document"]["tags"][0]["id"],
        original_tag.to_string()
    );
    assert_eq!(
        payload["source_event"]["event_id"],
        envelope.event_id().to_string()
    );
    assert_eq!(
        payload["source_event"]["mutation_id"],
        envelope.mutation_id().to_string()
    );
    assert_eq!(
        payload["occurred_at"],
        serde_json::to_value(envelope.occurred_at()).unwrap()
    );
    sqlx::query("DELETE FROM outbox_events WHERE id = ?")
        .bind(source.id)
        .execute(&h.pool)
        .await
        .unwrap();

    let outcome = with_actor(
        Some(UserId::from("must-not-be-inherited")),
        h.runner.execute(stored.id),
    )
    .await
    .unwrap();
    assert_eq!(outcome, RunOutcome::Succeeded);
    let emitted = h
        .pending_events()
        .await
        .into_iter()
        .find(|event| {
            event
                .envelope
                .as_ref()
                .is_some_and(|event| event.causation_id() == Some(envelope.event_id()))
        })
        .expect("workflow action must retain source lineage after outbox cleanup");
    assert_eq!(emitted.origin, EventOrigin::Workflow);
    let emitted_envelope = emitted.envelope.as_ref().unwrap();
    assert_eq!(emitted_envelope.correlation_id(), envelope.correlation_id());
    assert_eq!(emitted_envelope.actor_id(), None);
    h.engine.consume(&emitted).await.unwrap();
    assert_eq!(
        h.workflows
            .list_runs(h.vault, workflow.id, 50)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn event_actor_filters_use_the_producer_context_without_granting_private_access() {
    use folio_core::models::{UserId, WorkflowActorCondition, WorkflowEventConditions};
    use folio_core::mutation::with_actor;

    let h = Harness::new().await;
    let doc = h.document("actor.pdf", "application/pdf").await;
    let workflow = h
        .workflow(
            WorkflowTrigger::DocumentUpdated,
            WorkflowConditions {
                event: Some(WorkflowEventConditions {
                    subject_id: Some(doc.id),
                    actor: Some(WorkflowActorCondition::User {
                        user_id: "editor".into(),
                    }),
                }),
                ..Default::default()
            },
            unreachable_hook(),
        )
        .await;
    with_actor(
        Some(UserId::from("editor")),
        h.documents.update(
            h.vault,
            doc.id,
            UpdateDocument {
                title: Some("updated".into()),
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    let source = h
        .pending_events()
        .await
        .into_iter()
        .find(|event| {
            event
                .envelope
                .as_ref()
                .is_some_and(|envelope| envelope.event_type() == "document.updated")
        })
        .unwrap();
    let mut private = source.clone();
    let mut encoded = serde_json::to_value(source.envelope.as_ref().unwrap()).unwrap();
    encoded["audience"] = serde_json::json!({"type": "user", "user_id": "editor"});
    private.envelope = Some(serde_json::from_value(encoded).unwrap());
    assert!(h.engine.handle(&private).await.unwrap().is_empty());
    let runs = with_actor(Some(UserId::from("dispatcher")), h.engine.handle(&source))
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].workflow_id, workflow.id);
}
