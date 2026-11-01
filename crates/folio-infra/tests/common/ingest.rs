//! Shared setup for the ingestion tests: real SQLite, storage in a temp dir.

use std::sync::Arc;
use std::time::Duration;

use folio_core::models::{
    DocumentSource, EmailOrigin, FolderOrigin, Id, IngestItem, IngestOutcome, SourceConditions,
    WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowTrigger,
};
use folio_core::ports::{DocumentRepository, IngestLedger, IngestSink, WorkflowRepository};
use folio_core::services::{EventingDocumentRepository, Importer, IngestPipeline, WorkflowEngine};
use folio_core::EventBroadcaster;
use folio_infra::{Database, FileStorage, StorageFileMetadata};

pub struct Harness {
    pub pipeline: Arc<IngestPipeline>,
    pub ledger: Arc<dyn IngestLedger>,
    pub documents: Arc<dyn DocumentRepository>,
    pub workflows: Arc<dyn WorkflowRepository>,
    pub engine: WorkflowEngine,
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
        let dir = std::env::temp_dir().join(format!("folio-ingest-{}", Id::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let database = Database::connect(&format!("sqlite:{}/test.db?mode=rwc", dir.display()))
            .await
            .unwrap();
        database.migrate().await.unwrap();
        let repos = database.repositories().await;

        let events = EventBroadcaster::default();
        let documents = repos.documents;
        let eventing = Arc::new(EventingDocumentRepository::new(
            documents.clone(),
            events.clone(),
        ));
        let workflows = repos.workflows;
        let queue = repos.jobs;
        let ledger = repos.ledger;
        let pipeline = Arc::new(IngestPipeline::new(
            documents.clone(),
            repos.tags,
            workflows.clone(),
            ledger.clone(),
            Arc::new(Importer::new(
                eventing,
                Arc::new(FileStorage::new(&dir)),
                Arc::new(queue.clone()),
            )),
        ));
        let engine = WorkflowEngine::new(
            workflows.clone(),
            documents.clone(),
            repos.correspondents,
            repos.document_types,
            Arc::new(queue),
            Arc::new(StorageFileMetadata::new(FileStorage::new(&dir))),
            "http://folio.test",
        )
        .with_file_wait(Duration::ZERO);

        Self {
            pipeline,
            ledger,
            documents,
            workflows,
            engine,
            source_id: Id::new_v4(),
            dir,
        }
    }

    pub async fn ingest(&self, item: IngestItem) -> IngestOutcome {
        self.pipeline.ingest(self.source_id, item).await.unwrap()
    }

    pub async fn rule(&self, name: &str, source: SourceConditions) {
        self.workflows
            .create(WorkflowInput {
                name: name.into(),
                enabled: true,
                trigger: WorkflowTrigger::IngestReceived,
                conditions: WorkflowConditions {
                    source,
                    ..Default::default()
                },
                actions: vec![WorkflowAction::SkipImport],
            })
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
