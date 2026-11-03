//! Turns document events into workflow runs

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::{debug, error};

use crate::error::{Error, Result};
use crate::events::{DocumentEvent, EventOrigin, EventSnapshot, OutboxEvent};
use crate::models::{WebhookDocument, WebhookPayload, WorkflowRun, WorkflowTrigger};
use crate::ports::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, FileMetadata,
    WorkflowRepository,
};
use crate::DomainEventEnvelope;

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
            | DocumentEvent::PurgeRequested { .. }
            | DocumentEvent::Restored { .. } => None,
        }
    }

    /// Queue a run for every enabled workflow that matches the event
    ///
    /// A failure for one workflow does not stop the others. The first error
    /// is returned once all were tried, so the dispatcher retries the event;
    /// runs created in the meantime are not duplicated.
    pub async fn handle(&self, outbox_event: &OutboxEvent) -> Result<Vec<WorkflowRun>> {
        if outbox_event
            .envelope
            .as_ref()
            .is_some_and(|envelope| envelope.audience() != &crate::EventAudience::Vault)
        {
            return Ok(Vec::new());
        }
        let (trigger, document_id, ctx, occurred_at) = if let Some(event) = &outbox_event.event {
            let Some(trigger) = Self::trigger_for(event) else {
                return Ok(Vec::new());
            };
            (
                trigger,
                event.document_id(),
                self.load_context(outbox_event, event).await?,
                outbox_event.created_at,
            )
        } else if let Some(envelope) = &outbox_event.envelope {
            let Some(trigger) = WorkflowTrigger::parse(envelope.event_type()) else {
                return Ok(Vec::new());
            };
            if !matches!(
                trigger,
                WorkflowTrigger::DocumentTitleChanged
                    | WorkflowTrigger::DocumentTagsChanged
                    | WorkflowTrigger::DocumentCorrespondentChanged
                    | WorkflowTrigger::DocumentTypeChanged
            ) {
                return Ok(Vec::new());
            }
            let ctx = Self::metadata_context(envelope)?;
            (trigger, ctx.document_id, ctx, envelope.occurred_at())
        } else {
            return Ok(Vec::new());
        };
        let vault = ctx.vault_id;
        if outbox_event
            .envelope
            .as_ref()
            .is_some_and(|envelope| envelope.vault_id() != vault)
        {
            return Ok(Vec::new());
        }
        if trigger != WorkflowTrigger::DocumentDeleted
            && self.documents.locate(document_id).await? != Some(vault)
        {
            debug!(
                "Event {} document {document_id} is no longer active in its originating vault; skipping",
                outbox_event.id
            );
            return Ok(Vec::new());
        }
        let candidates = self.workflows.list_enabled(vault, trigger).await?;
        let mut payload_json: Option<String> = None;
        let mut runs = Vec::new();
        let mut first_error: Option<Error> = None;
        for workflow in candidates {
            let matches = match &outbox_event.envelope {
                Some(event) => workflow.conditions.matches_occurrence(Some(&ctx), event),
                None => workflow.conditions.matches_snapshot(&ctx),
            };
            if !matches {
                continue;
            }
            let outcome = async {
                let body = match &payload_json {
                    Some(body) => body.clone(),
                    None => {
                        let body = self
                            .build_payload(
                                trigger,
                                &ctx,
                                occurred_at,
                                outbox_event.envelope.as_ref(),
                            )
                            .await?;
                        payload_json = Some(body.clone());
                        body
                    }
                };
                let run = WorkflowRun::new(
                    workflow.vault_id,
                    workflow.id,
                    trigger,
                    Some(document_id),
                    body,
                )
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
                        trigger.as_str()
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

    fn metadata_context(envelope: &DomainEventEnvelope) -> Result<EventSnapshot> {
        let document_id = envelope.document_id().ok_or_else(|| {
            Error::Repository("Document metadata event has no document target".into())
        })?;
        if envelope.subject().subject_type != "document" || envelope.subject().id != document_id {
            return Err(Error::Repository(
                "Document metadata event target is invalid".into(),
            ));
        }
        let mut data = envelope.data().clone();
        data["document_id"] = serde_json::json!(document_id);
        data["vault_id"] = serde_json::json!(envelope.vault_id());
        data["file_path"] = serde_json::Value::Null;
        serde_json::from_value(data).map_err(|error| {
            Error::Repository(format!(
                "Document metadata event lacks a valid occurrence snapshot: {error}"
            ))
        })
    }

    async fn load_context(
        &self,
        outbox_event: &OutboxEvent,
        event: &DocumentEvent,
    ) -> Result<EventSnapshot> {
        if let Some(context) = &outbox_event.context {
            if context.document_id == event.document_id() {
                return Ok(context.as_ref().clone());
            }
        } else if let DocumentEvent::Deleted {
            document_id,
            snapshot: Some(snapshot),
        } = event
        {
            if snapshot.document.id == *document_id {
                let mut context = EventSnapshot::from(snapshot.as_ref());
                if let Some(id) = context.correspondent_id {
                    context.correspondent_name = self
                        .correspondents
                        .find_by_id(context.vault_id, id)
                        .await?
                        .map(|entry| entry.name);
                }
                if let Some(id) = context.document_type_id {
                    context.document_type_name = self
                        .document_types
                        .find_by_id(context.vault_id, id)
                        .await?
                        .map(|entry| entry.name);
                }
                return Ok(context);
            }
        }
        Err(Error::Repository(format!(
            "Cannot deliver outbox event {} ({}): historical document/vault context is missing or invalid; its original metadata cannot be reconstructed",
            outbox_event.id,
            event.event_type()
        )))
    }

    async fn build_payload(
        &self,
        trigger: WorkflowTrigger,
        ctx: &EventSnapshot,
        occurred_at: DateTime<Utc>,
        source_event: Option<&DomainEventEnvelope>,
    ) -> Result<String> {
        let size = match &ctx.file_path {
            Some(path) => self.files.size(path).await,
            None => None,
        };
        let mut payload = WebhookPayload::new(
            trigger,
            occurred_at,
            WebhookDocument::from_snapshot(ctx, size, &self.public_url),
        );
        payload.source_event = source_event.map(Into::into);
        serde_json::to_string(&payload)
            .map_err(|e| Error::Repository(format!("Failed to encode payload: {e}")))
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
