//! Turns document events into workflow runs

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::{broadcast, watch};
use tracing::{debug, error, warn};

use crate::error::Result;
use crate::events::DocumentEvent;
use crate::models::{
    Document, Id, Tag, VaultId, WebhookDocument, WebhookPayload, WorkflowRun, WorkflowTrigger,
};
use crate::ports::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, FileMetadata,
    WorkflowJobQueue, WorkflowRepository,
};

/// How long to wait for an upload to finish storing its file
const FILE_WAIT_DEFAULT: Duration = Duration::from_secs(3);
const FILE_POLL: Duration = Duration::from_millis(100);

/// Matches events against enabled workflows and queues a run for each hit
///
/// The engine only decides *what* runs; executing actions (with retries) is
/// the job worker's concern, so a slow webhook never blocks event handling.
pub struct WorkflowEngine {
    workflows: Arc<dyn WorkflowRepository>,
    documents: Arc<dyn DocumentRepository>,
    correspondents: Arc<dyn CorrespondentRepository>,
    document_types: Arc<dyn DocumentTypeRepository>,
    jobs: Arc<dyn WorkflowJobQueue>,
    files: Arc<dyn FileMetadata>,
    public_url: String,
    file_wait: Duration,
}

struct EventContext {
    document: Document,
    tags: Vec<Tag>,
}

impl WorkflowEngine {
    pub fn new(
        workflows: Arc<dyn WorkflowRepository>,
        documents: Arc<dyn DocumentRepository>,
        correspondents: Arc<dyn CorrespondentRepository>,
        document_types: Arc<dyn DocumentTypeRepository>,
        jobs: Arc<dyn WorkflowJobQueue>,
        files: Arc<dyn FileMetadata>,
        public_url: impl Into<String>,
    ) -> Self {
        Self {
            workflows,
            documents,
            correspondents,
            document_types,
            jobs,
            files,
            public_url: public_url.into(),
            file_wait: FILE_WAIT_DEFAULT,
        }
    }

    /// Override how long an upload may take to store its file
    pub fn with_file_wait(mut self, wait: Duration) -> Self {
        self.file_wait = wait;
        self
    }

    /// The trigger an event maps to; internal progress events map to none
    pub fn trigger_for(event: &DocumentEvent) -> Option<WorkflowTrigger> {
        match event {
            DocumentEvent::Created { .. } => Some(WorkflowTrigger::DocumentUploaded),
            DocumentEvent::Processed { .. } => Some(WorkflowTrigger::DocumentProcessed),
            DocumentEvent::Updated { .. } => Some(WorkflowTrigger::DocumentUpdated),
            DocumentEvent::Deleted { .. } => Some(WorkflowTrigger::DocumentDeleted),
            DocumentEvent::ThumbnailReady { .. } | DocumentEvent::OcrComplete { .. } => None,
        }
    }

    /// Consume events until the channel closes or shutdown is signalled
    pub async fn run(
        self: Arc<Self>,
        mut events: broadcast::Receiver<DocumentEvent>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                received = events.recv() => match received {
                    Ok(event) => {
                        // Handled off-loop: an upload may wait for its file to land.
                        let engine = Arc::clone(&self);
                        tokio::spawn(async move {
                            if let Err(e) = engine.handle(&event).await {
                                error!("Workflow engine failed on {}: {}", event.event_type(), e);
                            }
                        });
                    }
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        warn!("Workflow engine missed {missed} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    }

    /// Queue a run for every enabled workflow that matches the event
    pub async fn handle(&self, event: &DocumentEvent) -> Result<Vec<WorkflowRun>> {
        let Some(trigger) = Self::trigger_for(event) else {
            return Ok(Vec::new());
        };
        let document_id = event.document_id();
        let vault = match event {
            DocumentEvent::Deleted { snapshot, .. } => {
                snapshot.as_ref().map(|s| s.document.vault_id)
            }
            _ => self.documents.locate(document_id).await?,
        };
        let Some(vault) = vault else {
            debug!("Document {document_id} is gone, skipping {trigger:?}");
            return Ok(Vec::new());
        };
        let candidates = self.workflows.list_enabled(vault, trigger).await?;
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let Some(ctx) = self.load_context(vault, event, trigger).await? else {
            debug!("Document {document_id} is gone, skipping {trigger:?}");
            return Ok(Vec::new());
        };
        let tag_ids: Vec<Id> = ctx.tags.iter().map(|t| t.id).collect();

        let mut payload_json: Option<String> = None;
        let mut runs = Vec::new();
        for workflow in candidates {
            if !workflow.conditions.matches(&ctx.document, &tag_ids) {
                continue;
            }
            let body = match &payload_json {
                Some(body) => body.clone(),
                None => {
                    let body = self.build_payload(vault, trigger, &ctx).await?;
                    payload_json = Some(body.clone());
                    body
                }
            };
            let run = WorkflowRun::new(workflow.id, trigger, Some(document_id), body);
            if !self.workflows.create_run(&run).await? {
                debug!("Workflow {} already ran for {document_id}", workflow.id);
                continue;
            }
            self.jobs.enqueue_run(run.id).await?;
            runs.push(run);
        }
        Ok(runs)
    }

    async fn load_context(
        &self,
        vault: VaultId,
        event: &DocumentEvent,
        trigger: WorkflowTrigger,
    ) -> Result<Option<EventContext>> {
        if let DocumentEvent::Deleted { snapshot, .. } = event {
            return Ok(snapshot.as_ref().map(|s| EventContext {
                document: s.document.clone(),
                tags: s.tags.clone(),
            }));
        }

        let id = event.document_id();
        let mut document = self.documents.find_by_id(vault, id).await?;
        if trigger == WorkflowTrigger::DocumentUploaded {
            // An upload creates the record first and stores the file right
            // after; receivers expect the download link to work.
            let deadline = tokio::time::Instant::now() + self.file_wait;
            while let Some(doc) = &document {
                if doc.original_filename.is_none()
                    || doc.file_path.is_some()
                    || tokio::time::Instant::now() >= deadline
                {
                    break;
                }
                tokio::time::sleep(FILE_POLL).await;
                document = self.documents.find_by_id(vault, id).await?;
            }
        }

        match document {
            Some(document) => Ok(Some(EventContext {
                tags: self.documents.get_tags(vault, id).await?,
                document,
            })),
            None => Ok(None),
        }
    }

    async fn build_payload(
        &self,
        vault: VaultId,
        trigger: WorkflowTrigger,
        ctx: &EventContext,
    ) -> Result<String> {
        let doc = &ctx.document;
        let correspondent = match doc.correspondent_id {
            Some(id) => self.correspondents.find_by_id(vault, id).await?,
            None => None,
        };
        let document_type = match doc.document_type_id {
            Some(id) => self.document_types.find_by_id(vault, id).await?,
            None => None,
        };
        let size = match &doc.file_path {
            Some(path) => self.files.size(path).await,
            None => None,
        };
        let payload = WebhookPayload::new(
            trigger,
            Utc::now(),
            WebhookDocument::from_parts(
                doc,
                &ctx.tags,
                correspondent.as_ref(),
                document_type.as_ref(),
                size,
                &self.public_url,
            ),
        );
        serde_json::to_string(&payload)
            .map_err(|e| crate::error::Error::Repository(format!("Failed to encode payload: {e}")))
    }
}
