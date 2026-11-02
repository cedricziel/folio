//! Shared setup for the ingestion tests: real SQLite, storage in a temp dir.

use std::sync::Arc;

use folio_core::models::WorkflowRun;
use folio_core::models::{
    DocumentSource, EmailOrigin, FolderOrigin, Id, IngestItem, IngestOutcome, SourceConditions,
    WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowTrigger,
};
use folio_core::ports::DocumentStore;
use folio_core::ports::{
    DocumentImporter, DocumentRepository, IngestLedger, OutboxStore, VaultIngest,
    WorkflowRepository,
};
use folio_core::services::{Importer, IngestPipeline, WorkflowEngine};
use folio_core::DocumentEvent;
use folio_infra::{Database, ObjectStoreDocumentStore, StorageFileMetadata};

pub struct Harness {
    pub vault: folio_core::models::VaultId,
    pub pipeline: Arc<IngestPipeline>,
    pub ledger: Arc<dyn IngestLedger>,
    pub store: Arc<dyn DocumentStore>,
    pub documents: Arc<dyn DocumentRepository>,
    pub workflows: Arc<dyn WorkflowRepository>,
    pub engine: WorkflowEngine,
    pub outbox: Arc<dyn OutboxStore>,
    pub source_id: Id,
    pub dir: std::path::PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Harness {
    pub async fn new() -> Self {
        Self::with_importer(|importer| importer).await
    }

    pub async fn with_importer(
        wrap: impl FnOnce(Arc<dyn DocumentImporter>) -> Arc<dyn DocumentImporter>,
    ) -> Self {
        let dir = std::env::temp_dir().join(format!("folio-ingest-{}", Id::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let database = Database::connect(&format!("sqlite:{}/test.db?mode=rwc", dir.display()))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let repos = database.repositories().await;
        let vault = repos.vaults.instance_default().await.unwrap();

        let store: Arc<dyn DocumentStore> = Arc::new(ObjectStoreDocumentStore::in_memory());
        let documents = repos.documents;
        let workflows = repos.workflows;
        let queue = repos.jobs;
        let ledger = repos.ledger;
        let pipeline = Arc::new(IngestPipeline::new(
            documents.clone(),
            store.clone(),
            repos.tags,
            workflows.clone(),
            ledger.clone(),
            wrap(Arc::new(Importer::new(
                documents.clone(),
                store.clone(),
                Arc::new(queue.clone()),
            ))),
        ));
        let engine = WorkflowEngine::new(
            workflows.clone(),
            documents.clone(),
            repos.correspondents,
            repos.document_types,
            Arc::new(StorageFileMetadata::new(store.clone())),
            "http://folio.test",
        );

        Self {
            vault,
            pipeline,
            ledger,
            store,
            documents,
            workflows,
            engine,
            outbox: repos.outbox,
            source_id: Id::new_v4(),
            dir,
        }
    }

    /// Hand the import's `created` event from the outbox to the engine
    pub async fn handle_created(&self, document_id: Id) -> Vec<WorkflowRun> {
        let event = self
            .outbox
            .claim(100, chrono::Duration::seconds(30), 10)
            .await
            .unwrap()
            .into_iter()
            .find(|e| matches!(e.event, DocumentEvent::Created { document_id: d } if d == document_id))
            .expect("the import left a created event in the outbox");
        self.engine.handle(&event).await.unwrap()
    }

    pub async fn ingest(&self, item: IngestItem) -> IngestOutcome {
        self.pipeline
            .ingest(self.vault, self.source_id, item)
            .await
            .unwrap()
    }

    pub async fn rule(&self, name: &str, source: SourceConditions) {
        self.workflows
            .create(
                self.vault,
                WorkflowInput {
                    name: name.into(),
                    enabled: true,
                    trigger: WorkflowTrigger::IngestReceived,
                    conditions: WorkflowConditions {
                        source,
                        ..Default::default()
                    },
                    actions: vec![WorkflowAction::SkipImport],
                },
            )
            .await
            .unwrap();
    }
}

pub fn file_item(data: &[u8], tags: &[&str]) -> IngestItem {
    IngestItem {
        filename: "x.pdf".into(),
        mime_type: "application/pdf".into(),
        data: data.to_vec(),
        source: DocumentSource::folder(
            "inbox",
            FolderOrigin {
                relative_path: "Insurance/x.pdf".into(),
                original_filename: "x.pdf".into(),
            },
        ),
        tags: tags.iter().map(|t| t.to_string()).collect(),
    }
}

pub fn mail_item(from: &str, subject: &str, attachment: &str, data: &[u8]) -> IngestItem {
    IngestItem {
        filename: attachment.into(),
        mime_type: "application/pdf".into(),
        data: data.to_vec(),
        source: DocumentSource::email(
            "work",
            EmailOrigin {
                account: "work".into(),
                mailbox: "INBOX".into(),
                message_id: "m1@test".into(),
                from: from.into(),
                to: vec!["me@home.test".into()],
                subject: subject.into(),
                date: None,
                attachment_filename: Some(attachment.into()),
                attachment_content_type: Some("application/pdf".into()),
            },
        ),
        tags: Vec::new(),
    }
}
