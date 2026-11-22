//! Turns document events into workflow runs

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::{debug, error};

use super::IngestWorkflowAccess;
use crate::error::{Error, Result};
use crate::events::{DocumentEvent, EventOrigin, EventSnapshot, OutboxEvent};
use crate::models::{
    WebhookDocument, WebhookEntry, WebhookIngest, WebhookMetadata, WebhookPayload, WorkflowRun,
    WorkflowTarget, WorkflowTrigger, WorkflowTriggerRegistry,
};
use crate::ports::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, FileMetadata,
    WorkflowRepository,
};
use crate::{DomainEventEnvelope, EventAudience};

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
    ingest_access: Option<Arc<IngestWorkflowAccess>>,
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
            ingest_access: None,
        }
    }

    pub fn with_ingest_access(mut self, access: Arc<IngestWorkflowAccess>) -> Self {
        self.ingest_access = Some(access);
        self
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
        if matches!(
            outbox_event.origin,
            EventOrigin::Workflow | EventOrigin::Import
        ) {
            return Ok(Vec::new());
        }
        if let Some(envelope) = &outbox_event.envelope {
            match envelope.audience() {
                EventAudience::Owners => {
                    return self.handle_ingest_outcome(outbox_event.id, envelope).await
                }
                EventAudience::User { .. } => return Ok(Vec::new()),
                EventAudience::Vault => {}
            }
        }
        let legacy = outbox_event.event.as_ref().and_then(Self::trigger_for);
        let envelope_type = outbox_event
            .envelope
            .as_ref()
            .map(DomainEventEnvelope::event_type);
        let candidates = WorkflowTriggerRegistry::built_in()
            .resolve_candidates(envelope_type, legacy.as_ref().map(WorkflowTrigger::as_str));
        let mut runs = Vec::new();
        let mut first_error = None;
        for definition in candidates {
            let trigger = WorkflowTrigger::parse(&definition.event_type)
                .expect("built-in trigger has a wire name");
            match self
                .handle_candidate(outbox_event, trigger, legacy == Some(trigger))
                .await
            {
                Ok(candidate_runs) => runs.extend(candidate_runs),
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(runs), Err)
    }

    async fn handle_ingest_outcome(
        &self,
        outbox_id: i64,
        envelope: &DomainEventEnvelope,
    ) -> Result<Vec<WorkflowRun>> {
        let (Some(access), Some(trigger)) = (
            &self.ingest_access,
            WorkflowTrigger::parse(envelope.event_type())
                .filter(|trigger| trigger.is_ingest_outcome()),
        ) else {
            return Ok(Vec::new());
        };
        let ingest: WebhookIngest = serde_json::from_value(envelope.data().clone())
            .map_err(|error| Error::Repository(format!("Invalid ingestion occurrence: {error}")))?;
        let valid_subject = match ingest.outcome {
            Some(outcome) => {
                outcome.event_type() == envelope.event_type()
                    && ingest.item_id == Some(envelope.subject().id)
                    && ingest.attempt_id == Some(envelope.mutation_id())
                    && ingest.reason_code.as_deref() == Some(outcome.reason_code())
                    && ingest.retryable == Some(outcome.retryable())
            }
            None => {
                ingest.source_id == envelope.subject().id
                    && envelope.subject().subject_type == "source"
                    && ingest.item_id.is_none()
                    && ingest.attempt_id.is_none()
                    && ingest.retryable.is_none()
                    && ingest.reason_code.as_deref()
                        == (trigger == WorkflowTrigger::SourceFailed)
                            .then_some("source_unavailable")
            }
        };
        let vault = envelope.vault_id();
        if !valid_subject
            || ingest.payload_version != 1
            || envelope.document_id().is_some()
            || !access
                .permits(vault, ingest.source_id, ingest.source_kind)
                .await?
        {
            return Ok(Vec::new());
        }
        let body = serde_json::to_string(&WebhookPayload::for_ingest(
            trigger,
            envelope.occurred_at(),
            ingest,
            Some(envelope.into()),
        ))
        .map_err(|error| Error::Repository(format!("Failed to encode payload: {error}")))?;
        let mut runs = Vec::new();
        let mut first_error = None;
        for workflow in self.workflows.list_enabled(vault, trigger).await? {
            if !workflow
                .actions
                .iter()
                .all(|action| trigger.target().accepts(action))
                || !workflow.conditions.matches_occurrence(None, envelope)
            {
                continue;
            }
            let run = WorkflowRun::new(vault, workflow.id, trigger, None, body.clone())
                .with_actions(workflow.actions.clone())
                .with_source_event(outbox_id);
            match self.workflows.create_run(&run).await {
                Ok(true) => runs.push(run),
                Ok(false) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(runs), Err)
    }

    async fn handle_candidate(
        &self,
        outbox_event: &OutboxEvent,
        trigger: WorkflowTrigger,
        use_projection: bool,
    ) -> Result<Vec<WorkflowRun>> {
        let (document_id, vault, snapshot, occurred_at) = if use_projection {
            let event = outbox_event
                .event
                .as_ref()
                .expect("legacy trigger has a projection");
            let ctx = self.load_context(outbox_event, event).await?;
            (
                Some(event.document_id()),
                ctx.vault_id,
                Some(ctx),
                outbox_event.created_at,
            )
        } else if let Some(envelope) = &outbox_event.envelope {
            if trigger.entry_transition().is_some() {
                let Some(document_id) = envelope.document_id() else {
                    return Ok(Vec::new());
                };
                (
                    Some(document_id),
                    envelope.vault_id(),
                    None,
                    envelope.occurred_at(),
                )
            } else if trigger.is_custom_metadata() {
                if !Self::valid_metadata_subject(trigger, envelope) {
                    return Ok(Vec::new());
                }
                (
                    envelope.document_id(),
                    envelope.vault_id(),
                    None,
                    envelope.occurred_at(),
                )
            } else if matches!(
                trigger,
                WorkflowTrigger::DocumentTitleChanged
                    | WorkflowTrigger::DocumentTagsChanged
                    | WorkflowTrigger::DocumentCorrespondentChanged
                    | WorkflowTrigger::DocumentTypeChanged
            ) {
                let ctx = Self::metadata_context(envelope)?;
                (
                    Some(ctx.document_id),
                    ctx.vault_id,
                    Some(ctx),
                    envelope.occurred_at(),
                )
            } else {
                return Ok(Vec::new());
            }
        } else {
            return Ok(Vec::new());
        };
        if outbox_event
            .envelope
            .as_ref()
            .is_some_and(|envelope| envelope.vault_id() != vault)
        {
            return Ok(Vec::new());
        }
        if trigger.target() == WorkflowTarget::Document {
            let Some(id) = document_id else {
                return Ok(Vec::new());
            };
            if self.documents.locate(id).await? != Some(vault) {
                debug!(
                    "Event {} document {id} is no longer active in its originating vault; skipping",
                    outbox_event.id
                );
                return Ok(Vec::new());
            }
        }
        let candidates = self.workflows.list_enabled(vault, trigger).await?;
        let mut payload_json: Option<String> = None;
        let mut runs = Vec::new();
        let mut first_error: Option<Error> = None;
        for workflow in candidates {
            let matches = match &outbox_event.envelope {
                Some(event) => workflow
                    .conditions
                    .matches_occurrence(snapshot.as_ref(), event),
                None => snapshot
                    .as_ref()
                    .is_some_and(|ctx| workflow.conditions.matches_snapshot(ctx)),
            };
            if !matches
                || !workflow
                    .actions
                    .iter()
                    .all(|action| trigger.target().accepts(action))
            {
                continue;
            }
            let outcome = async {
                let body = match &payload_json {
                    Some(body) => body.clone(),
                    None => {
                        let envelope = outbox_event.envelope.as_ref();
                        let body = if let Some(ctx) = &snapshot {
                            self.build_payload(trigger, ctx, occurred_at, envelope)
                                .await?
                        } else if trigger.is_custom_metadata() {
                            let source =
                                envelope.expect("metadata occurrence has a domain envelope");
                            let payload = WebhookPayload::for_metadata(
                                trigger,
                                occurred_at,
                                WebhookMetadata {
                                    vault_id: vault,
                                    subject_id: source.subject().id,
                                    document_id,
                                    actor_id: source.actor_id().cloned(),
                                    causation_id: source.causation_id(),
                                    data: source.data().clone(),
                                },
                                Some(source.into()),
                            );
                            serde_json::to_string(&payload).map_err(|e| {
                                Error::Repository(format!("Failed to encode payload: {e}"))
                            })?
                        } else {
                            let payload = envelope
                                .and_then(|source| Self::entry_payload(trigger, source))
                                .ok_or_else(|| {
                                    Error::Repository("Invalid workflow entry occurrence".into())
                                })?;
                            serde_json::to_string(&payload).map_err(|e| {
                                Error::Repository(format!("Failed to encode payload: {e}"))
                            })?
                        };
                        payload_json = Some(body.clone());
                        body
                    }
                };
                let run =
                    WorkflowRun::new(workflow.vault_id, workflow.id, trigger, document_id, body)
                        .with_actions(workflow.actions.clone())
                        .with_source_event(outbox_event.id);
                Ok::<_, Error>(self.workflows.create_run(&run).await?.then_some(run))
            }
            .await;
            match outcome {
                Ok(Some(run)) => runs.push(run),
                Ok(None) => debug!(
                    "Workflow {} already handled event {}",
                    workflow.id, outbox_event.id
                ),
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

    fn valid_metadata_subject(trigger: WorkflowTrigger, event: &DomainEventEnvelope) -> bool {
        let subject = event.subject();
        let expected_type = trigger.as_str().split('.').next();
        if Some(subject.subject_type.as_str()) != expected_type {
            return false;
        }
        let id_matches = |key: &str| {
            event.data()[key]
                .as_str()
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                == Some(subject.id)
        };
        match subject.subject_type.as_str() {
            "document" => event.document_id() == Some(subject.id),
            "metadata_field" => event.document_id().is_none() && id_matches("field_id"),
            "metadata_option" => event.document_id().is_none() && id_matches("option_id"),
            _ => false,
        }
    }

    fn entry_payload(
        trigger: WorkflowTrigger,
        source: &DomainEventEnvelope,
    ) -> Option<WebhookPayload> {
        let (entry_kind, transition) = trigger.entry_transition()?;
        let subject = source.subject();
        let expected_subject = match entry_kind {
            crate::models::EntryKind::Note => "document_note",
            crate::models::EntryKind::Comment => "document_comment",
        };
        let data = source.data();
        if subject.subject_type != expected_subject
            || data["entry_kind"].as_str() != Some(entry_kind.as_str())
            || data["transition"].as_str() != Some(transition)
            || data["entry_id"]
                .as_str()
                .and_then(|id| uuid::Uuid::parse_str(id).ok())
                != Some(subject.id)
        {
            return None;
        }
        Some(WebhookPayload::for_entry(
            trigger,
            source.occurred_at(),
            WebhookEntry {
                entry_id: subject.id,
                document_id: source.document_id()?,
                vault_id: source.vault_id(),
                entry_kind,
                transition: transition.into(),
                actor_id: source.actor_id().cloned(),
                causation_id: source.causation_id(),
            },
            Some(source.into()),
        ))
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
        self.handle(event).await.map(|_| ())
    }
}
