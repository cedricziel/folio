//! Executes queued workflow runs

use std::sync::Arc;

use chrono::Utc;
use tracing::{info, warn};

use folio_core::error::Error;
use folio_core::models::{
    validate_webhook_url, Id, RunStatus, UpdateDocument, VaultId, ViewVisibility, WebhookPayload,
    Workflow, WorkflowAction, WorkflowRun, WorkflowTarget, WorkflowTrigger,
};
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::ports::{DocumentRepository, SavedViewRepository, WorkflowRepository};
use folio_core::services::IngestWorkflowAccess;

use super::webhook::{WebhookClient, WebhookRequest};

/// Attempts per run before it is marked failed
pub const MAX_ATTEMPTS: u32 = 5;

/// What the caller should do with the job that triggered the execution
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// Every action finished
    Succeeded,
    /// An action failed in a way that may heal; run the job again later
    Retry,
    /// The run is over and will not be retried
    Failed,
}

struct ActionFailure {
    message: String,
    retryable: bool,
    response_status: Option<u16>,
}

/// Runs the actions of a [`WorkflowRun`], recording progress and errors on it
///
/// Actions run in order and each is at-least-once: a run that is retried
/// resumes at the action that failed rather than repeating finished ones. It
/// executes the actions snapshotted when the run was created, so editing the
/// workflow in between changes nothing. Document changes go through a
/// repository whose events are attributed to workflows: live clients see them,
/// but they never trigger `document.updated` workflows again.
pub struct WorkflowRunner {
    workflows: Arc<dyn WorkflowRepository>,
    documents: Arc<dyn DocumentRepository>,
    client: WebhookClient,
    ingest_access: Option<Arc<IngestWorkflowAccess>>,
    saved_views: Option<Arc<dyn SavedViewRepository>>,
}

impl WorkflowRunner {
    pub fn new(
        workflows: Arc<dyn WorkflowRepository>,
        documents: Arc<dyn DocumentRepository>,
        client: WebhookClient,
    ) -> Self {
        Self {
            workflows,
            documents,
            client,
            ingest_access: None,
            saved_views: None,
        }
    }

    pub fn with_ingest_access(mut self, access: Arc<IngestWorkflowAccess>) -> Self {
        self.ingest_access = Some(access);
        self
    }

    pub fn with_saved_views(mut self, repo: Arc<dyn SavedViewRepository>) -> Self {
        self.saved_views = Some(repo);
        self
    }

    /// Make one attempt at a run
    pub async fn execute(&self, run_id: Id) -> folio_core::Result<RunOutcome> {
        let Some(mut run) = self.workflows.find_run(run_id).await? else {
            warn!("Workflow run {run_id} no longer exists");
            return Ok(RunOutcome::Failed);
        };
        if run.status.is_terminal() {
            return Ok(match run.status {
                RunStatus::Succeeded => RunOutcome::Succeeded,
                _ => RunOutcome::Failed,
            });
        }

        let vault = run.vault_id;
        if self.active_definition(&run).await?.is_none() {
            self.finish(
                &mut run,
                RunStatus::Failed,
                Some("Workflow definition is no longer active".into()),
            )
            .await?;
            return Ok(RunOutcome::Failed);
        }

        let ingest = if run.event.is_ingest_outcome() {
            let payload: WebhookPayload = serde_json::from_str(&run.payload).map_err(|error| {
                Error::Repository(format!("Invalid ingestion workflow payload: {error}"))
            })?;
            let Some(ingest) = payload.ingest.filter(|_| payload.event == run.event) else {
                self.finish(
                    &mut run,
                    RunStatus::Failed,
                    Some("Missing ingestion workflow context".into()),
                )
                .await?;
                return Ok(RunOutcome::Failed);
            };
            Some(ingest)
        } else {
            None
        };
        let saved_view = if run.event.is_saved_view() {
            let payload: WebhookPayload = serde_json::from_str(&run.payload).map_err(|error| {
                Error::Repository(format!("Invalid saved view workflow payload: {error}"))
            })?;
            let Some(view) = payload.saved_view.filter(|view| {
                payload.event == run.event
                    && view.vault_id == vault
                    && view.visibility == ViewVisibility::Shared
                    && payload.source_event.is_some()
                    && payload.document.is_none()
                    && run.document_id.is_none()
            }) else {
                self.finish(
                    &mut run,
                    RunStatus::Failed,
                    Some("Missing saved view workflow context".into()),
                )
                .await?;
                return Ok(RunOutcome::Failed);
            };
            Some(view)
        } else {
            None
        };
        let lineage = run.source_identity()?.map(|source| MutationLineage {
            correlation_id: source.correlation_id,
            causation_id: Some(source.event_id),
        });
        run.attempts += 1;
        let skip = run.completed_actions as usize;
        let actions = run.actions.clone();
        for (index, action) in actions.iter().enumerate().skip(skip) {
            let Some(stored) = self.workflows.find_run(run.id).await? else {
                return Ok(RunOutcome::Failed);
            };
            if stored.status.is_terminal() {
                return Ok(if stored.status == RunStatus::Succeeded {
                    RunOutcome::Succeeded
                } else {
                    RunOutcome::Failed
                });
            }
            let Some(workflow) = self.active_definition(&run).await? else {
                self.finish(
                    &mut run,
                    RunStatus::Failed,
                    Some("Workflow definition is no longer active".into()),
                )
                .await?;
                return Ok(RunOutcome::Failed);
            };
            if run.event.target() == WorkflowTarget::Document {
                if let Some(document_id) = run.document_id {
                    if self.documents.locate(document_id).await? != Some(vault) {
                        self.finish(
                            &mut run,
                            RunStatus::Failed,
                            Some("Document is no longer active in the workflow's vault".into()),
                        )
                        .await?;
                        return Ok(RunOutcome::Failed);
                    }
                }
            }
            if let Some(ingest) = &ingest {
                let allowed = if let Some(access) = &self.ingest_access {
                    access
                        .permits(vault, ingest.source_id, ingest.source_kind)
                        .await?
                        && workflow.trigger == run.event
                        && workflow.actions == run.actions
                        && run.event.target().accepts(action)
                } else {
                    false
                };
                if !allowed {
                    self.finish(
                        &mut run,
                        RunStatus::Failed,
                        Some("Source or workflow is no longer authorized".into()),
                    )
                    .await?;
                    return Ok(RunOutcome::Failed);
                }
            }
            if let Some(view) = &saved_view {
                let allowed = if let Some(repo) = &self.saved_views {
                    run.event.target().accepts(action)
                        && repo
                            .is_shared_for_workflow(
                                vault,
                                view.saved_view_id,
                                run.event == WorkflowTrigger::SavedViewDeleted,
                            )
                            .await?
                } else {
                    false
                };
                if !allowed {
                    self.finish(
                        &mut run,
                        RunStatus::Failed,
                        Some("Saved view is no longer shared in the workflow's vault".into()),
                    )
                    .await?;
                    return Ok(RunOutcome::Failed);
                }
            }
            match with_actor(
                None,
                with_lineage(lineage, self.perform(vault, index, action, &run)),
            )
            .await
            {
                Ok(response_status) => {
                    run.completed_actions = index as u32 + 1;
                    run.response_status = response_status.or(run.response_status);
                    if !self.save(&run).await? {
                        return Ok(RunOutcome::Failed);
                    }
                }
                Err(failure) => {
                    run.response_status = failure.response_status;
                    let give_up = !failure.retryable || run.attempts >= MAX_ATTEMPTS;
                    if give_up {
                        self.finish(&mut run, RunStatus::Failed, Some(failure.message))
                            .await?;
                        return Ok(RunOutcome::Failed);
                    }
                    run.last_error = Some(failure.message);
                    if !self.save(&run).await? {
                        return Ok(RunOutcome::Failed);
                    }
                    return Ok(RunOutcome::Retry);
                }
            }
        }

        if !self.finish(&mut run, RunStatus::Succeeded, None).await? {
            return Ok(RunOutcome::Failed);
        }
        info!(
            "Workflow run {} succeeded after {} attempt(s)",
            run.id, run.attempts
        );
        Ok(RunOutcome::Succeeded)
    }

    /// Mark a run failed because its job gave up, unless it already ended
    pub async fn abandon(&self, run_id: Id, reason: &str) -> folio_core::Result<()> {
        let Some(mut run) = self.workflows.find_run(run_id).await? else {
            return Ok(());
        };
        if run.status.is_terminal() {
            return Ok(());
        }
        // Keep the action's own error if there is one; it says more than the job's.
        let error = run.last_error.take().unwrap_or_else(|| reason.to_string());
        self.finish(&mut run, RunStatus::Failed, Some(error))
            .await
            .map(|_| ())
    }

    async fn active_definition(&self, run: &WorkflowRun) -> folio_core::Result<Option<Workflow>> {
        Ok(self
            .workflows
            .find_by_id(run.vault_id, run.workflow_id)
            .await?
            .filter(|workflow| {
                workflow.enabled
                    && run
                        .definition_revision
                        .is_none_or(|revision| revision == workflow.revision)
            }))
    }

    async fn finish(
        &self,
        run: &mut WorkflowRun,
        status: RunStatus,
        error: Option<String>,
    ) -> folio_core::Result<bool> {
        run.status = status;
        run.last_error = error;
        run.finished_at = Some(Utc::now());
        self.save(run).await
    }

    /// Persists progress; false when the run was stopped elsewhere, such as by deleting its workflow
    async fn save(&self, run: &WorkflowRun) -> folio_core::Result<bool> {
        match self.workflows.update_run(run).await {
            Ok(()) => Ok(true),
            Err(folio_core::Error::Conflict(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Returns the webhook's HTTP status when the action was a webhook
    async fn perform(
        &self,
        vault: VaultId,
        action_index: usize,
        action: &WorkflowAction,
        run: &WorkflowRun,
    ) -> Result<Option<u16>, ActionFailure> {
        match action {
            WorkflowAction::Webhook {
                url,
                headers,
                secret,
                ..
            } => {
                validate_webhook_url(url).map_err(|e| ActionFailure {
                    message: e.to_string(),
                    retryable: false,
                    response_status: None,
                })?;
                let result = self
                    .client
                    .send(WebhookRequest {
                        url,
                        headers,
                        secret: secret.as_deref(),
                        event: run.event,
                        run_id: run.id,
                        action_index,
                        body: &run.payload,
                    })
                    .await;
                match result.error {
                    None => Ok(result.status),
                    Some(message) => Err(ActionFailure {
                        message,
                        retryable: result.retryable,
                        response_status: result.status,
                    }),
                }
            }
            local => self.perform_local(vault, local, run).await.map(|()| None),
        }
    }

    async fn perform_local(
        &self,
        vault: VaultId,
        action: &WorkflowAction,
        run: &WorkflowRun,
    ) -> Result<(), ActionFailure> {
        let document_id = run.document_id.ok_or_else(|| ActionFailure {
            message: "Run has no document to change".into(),
            retryable: false,
            response_status: None,
        })?;

        let result = match action {
            WorkflowAction::AddTags { tag_ids } => {
                let mut result = Ok(());
                for tag_id in tag_ids {
                    result = self.documents.add_tag(vault, document_id, *tag_id).await;
                    if result.is_err() {
                        break;
                    }
                }
                result
            }
            WorkflowAction::SetCorrespondent { correspondent_id } => self
                .documents
                .update(
                    vault,
                    document_id,
                    UpdateDocument {
                        correspondent_id: Some(Some(*correspondent_id)),
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ()),
            WorkflowAction::SetDocumentType { document_type_id } => self
                .documents
                .update(
                    vault,
                    document_id,
                    UpdateDocument {
                        document_type_id: Some(Some(*document_type_id)),
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ()),
            WorkflowAction::Webhook { .. } => unreachable!("webhooks are handled by the caller"),
            // Only valid before import, where the pipeline handles it; nothing to do on a stored document.
            WorkflowAction::SkipImport => Ok(()),
        };

        result.map_err(|e| ActionFailure {
            retryable: !matches!(e, Error::NotFound { .. } | Error::Validation(_)),
            message: e.to_string(),
            response_status: None,
        })
    }
}
