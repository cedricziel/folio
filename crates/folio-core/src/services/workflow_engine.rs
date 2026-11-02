//! Turns document events into workflow runs

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tracing::{debug, error};

use crate::error::{Error, Result};
use crate::events::{DocumentEvent, EventOrigin, OutboxEvent};
use crate::models::{
    Document, Id, Tag, VaultId, WebhookDocument, WebhookPayload, WorkflowRun, WorkflowTrigger,
};
use crate::ports::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, FileMetadata,
    WorkflowRepository,
};

use super::outbox_dispatcher::EventConsumer;

/// Matches events against enabled workflows and queues a run for each hit
///
/// The engine only decides *what* runs; executing actions (with retries) is
/// the job worker's concern, so a slow webhook never blocks event handling.
/// It is fed by the outbox dispatcher, so every committed event is seen even
/// across restarts. Handling an event twice creates no second run.
pub struct WorkflowEngine {
    workflows: Arc<dyn WorkflowRepository>,
    documents: Arc<dyn DocumentRepository>,
    correspondents: Arc<dyn CorrespondentRepository>,
    document_types: Arc<dyn DocumentTypeRepository>,
    files: Arc<dyn FileMetadata>,
    public_url: String,
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
        files: Arc<dyn FileMetadata>,
        public_url: impl Into<String>,
    ) -> Self {
        Self {
            workflows,
            documents,
            correspondents,
            document_types,
            files,
            public_url: public_url.into(),
        }
    }

    /// The trigger an event maps to; internal progress events map to none
    pub fn trigger_for(event: &DocumentEvent) -> Option<WorkflowTrigger> {
        match event {
            DocumentEvent::Created { .. } => Some(WorkflowTrigger::DocumentUploaded),
            DocumentEvent::Processed { .. } => Some(WorkflowTrigger::DocumentProcessed),
            DocumentEvent::Updated { .. } => Some(WorkflowTrigger::DocumentUpdated),
            DocumentEvent::Deleted { .. } => Some(WorkflowTrigger::DocumentDeleted),
            DocumentEvent::ThumbnailReady { .. }
            | DocumentEvent::OcrComplete { .. }
            | DocumentEvent::Trashed { .. }
            | DocumentEvent::Restored { .. } => None,
        }
    }

    /// Queue a run for every enabled workflow that matches the event
    ///
    /// A failure for one workflow does not stop the others. The first error
    /// is returned once all were tried, so the dispatcher retries the event;
    /// runs created in the meantime are not duplicated.
    pub async fn handle(&self, outbox_event: &OutboxEvent) -> Result<Vec<WorkflowRun>> {
        let event = &outbox_event.event;
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

        let Some(ctx) = self.load_context(vault, event).await? else {
            debug!("Document {document_id} is gone, skipping {trigger:?}");
            return Ok(Vec::new());
        };
        let tag_ids: Vec<Id> = ctx.tags.iter().map(|t| t.id).collect();

        let mut payload_json: Option<String> = None;
        let mut runs = Vec::new();
        let mut first_error: Option<Error> = None;
        for workflow in candidates {
            if !workflow.conditions.matches(&ctx.document, &tag_ids) {
                continue;
            }
            let outcome = async {
                let body = match &payload_json {
                    Some(body) => body.clone(),
                    None => {
                        let body = self.build_payload(vault, trigger, &ctx).await?;
                        payload_json = Some(body.clone());
                        body
                    }
                };
                let run = WorkflowRun::new(workflow.id, trigger, Some(document_id), body)
                    .with_actions(workflow.actions.clone())
                    .with_source_event(outbox_event.id);
                Ok::<_, Error>(self.workflows.create_run(&run).await?.then_some(run))
            }
            .await;
            match outcome {
                Ok(Some(run)) => runs.push(run),
                Ok(None) => debug!("Workflow {} already ran for {document_id}", workflow.id),
                Err(e) => {
                    error!(
                        "Workflow {} failed on {}: {e}",
                        workflow.id,
                        event.event_type()
                    );
                    first_error.get_or_insert(e);
                }
            }
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(runs),
        }
    }

    async fn load_context(
        &self,
        vault: VaultId,
        event: &DocumentEvent,
    ) -> Result<Option<EventContext>> {
        if let DocumentEvent::Deleted { snapshot, .. } = event {
            return Ok(snapshot.as_ref().map(|s| EventContext {
                document: s.document.clone(),
                tags: s.tags.clone(),
            }));
        }

        let id = event.document_id();
        let document = self.documents.find_by_id(vault, id).await?;

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

#[async_trait]
impl EventConsumer for WorkflowEngine {
    async fn consume(&self, event: &OutboxEvent) -> Result<()> {
        if event.origin == EventOrigin::Workflow {
            return Ok(());
        }
        self.handle(event).await.map(|_| ())
    }
}
