//! Executes queued workflow runs

use std::sync::Arc;

use chrono::Utc;
use tracing::{info, warn};

use folio_core::error::Error;
use folio_core::models::{
    validate_webhook_url, Id, RunStatus, ViewVisibility, WebhookPayload, Workflow, WorkflowAction,
    WorkflowRun, WorkflowTarget, WorkflowTrigger,
};
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::ports::{DocumentRepository, SavedViewRepository, WorkflowRepository};
use folio_core::services::IngestWorkflowAccess;

use super::webhook::{WebhookClient, WebhookRequest};

/// Attempts per run before it is marked failed
pub const MAX_ATTEMPTS: u32 = 5;

const DOCUMENT_UNAVAILABLE: &str = "Document is no longer active in the workflow's vault";

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

enum ActionProgress {
    Webhook(Option<u16>),
    Document { run_finished: bool },
}

struct ActionFailure {
    message: String,
    retryable: bool,
    response_status: Option<u16>,
}

/// Runs the actions of a [`WorkflowRun`], recording progress and errors on it
///
/// A retry resumes from persisted progress. Document changes and their progress
/// commit together, while webhooks use a stable delivery ID across retries.
/// Each run holds the actions captured when it was queued.
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
            return self
                .finish(
                    &mut run,
                    RunStatus::Failed,
                    Some("Workflow definition is no longer active".into()),
                )
                .await;
        }

        let ingest = if run.event.is_ingest_outcome() {
            let payload: WebhookPayload = serde_json::from_str(&run.payload).map_err(|error| {
                Error::Repository(format!("Invalid ingestion workflow payload: {error}"))
            })?;
            let Some(ingest) = payload.ingest.filter(|_| payload.event == run.event) else {
                return self
                    .finish(
                        &mut run,
                        RunStatus::Failed,
                        Some("Missing ingestion workflow context".into()),
                    )
                    .await;
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
                return self
                    .finish(
                        &mut run,
                        RunStatus::Failed,
                        Some("Missing saved view workflow context".into()),
                    )
                    .await;
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
            if advance_to_persisted_progress(&mut run, &stored, index) {
                continue;
            }
            let Some(workflow) = self.active_definition(&run).await? else {
                return self
                    .finish(
                        &mut run,
                        RunStatus::Failed,
                        Some("Workflow definition is no longer active".into()),
                    )
                    .await;
            };
            if action.is_webhook() && run.event.target() == WorkflowTarget::Document {
                if let Some(document_id) = run.document_id {
                    if self.documents.locate(document_id).await? != Some(vault) {
                        return self
                            .finish(
                                &mut run,
                                RunStatus::Failed,
                                Some(DOCUMENT_UNAVAILABLE.into()),
                            )
                            .await;
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
                    return self
                        .finish(
                            &mut run,
                            RunStatus::Failed,
                            Some("Source or workflow is no longer authorized".into()),
                        )
                        .await;
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
                    return self
                        .finish(
                            &mut run,
                            RunStatus::Failed,
                            Some("Saved view is no longer shared in the workflow's vault".into()),
                        )
                        .await;
                }
            }
            match with_actor(
                None,
                with_lineage(lineage, self.perform(index, action, &run)),
            )
            .await
            {
                Ok(ActionProgress::Webhook(response_status)) => {
                    run.completed_actions = index as u32 + 1;
                    run.response_status = response_status.or(run.response_status);
                    if let Some(outcome) = self.save(&run).await? {
                        return Ok(outcome);
                    }
                }
                Ok(ActionProgress::Document { run_finished }) => {
                    run.completed_actions = index as u32 + 1;
                    if run_finished {
                        info!(
                            "Workflow run {} succeeded after {} attempt(s)",
                            run.id, run.attempts
                        );
                        return Ok(RunOutcome::Succeeded);
                    }
                }
                Err(failure) => {
                    let stored = if action.is_webhook() {
                        None
                    } else {
                        self.workflows.find_run(run.id).await?
                    };
                    if let Some(stored) = stored {
                        if stored.status.is_terminal() {
                            return Ok(match stored.status {
                                RunStatus::Succeeded => RunOutcome::Succeeded,
                                _ => RunOutcome::Failed,
                            });
                        }
                        if advance_to_persisted_progress(&mut run, &stored, index) {
                            continue;
                        }
                    }
                    run.response_status = failure.response_status;
                    let give_up = !failure.retryable || run.attempts >= MAX_ATTEMPTS;
                    if give_up {
                        return self
                            .finish(&mut run, RunStatus::Failed, Some(failure.message))
                            .await;
                    }
                    run.last_error = Some(failure.message);
                    if let Some(outcome) = self.save(&run).await? {
                        return Ok(outcome);
                    }
                    return Ok(RunOutcome::Retry);
                }
            }
        }

        let outcome = self.finish(&mut run, RunStatus::Succeeded, None).await?;
        if outcome == RunOutcome::Succeeded {
            info!(
                "Workflow run {} succeeded after {} attempt(s)",
                run.id, run.attempts
            );
        }
        Ok(outcome)
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
    ) -> folio_core::Result<RunOutcome> {
        run.status = status;
        run.last_error = error;
        run.finished_at = Some(Utc::now());
        Ok(self.save(run).await?.unwrap_or(match status {
            RunStatus::Pending => RunOutcome::Retry,
            RunStatus::Succeeded => RunOutcome::Succeeded,
            RunStatus::Failed => RunOutcome::Failed,
        }))
    }

    async fn save(&self, run: &WorkflowRun) -> folio_core::Result<Option<RunOutcome>> {
        match self.workflows.update_run(run).await {
            Ok(()) => Ok(None),
            Err(folio_core::Error::Conflict(_)) => {
                let stored = self.workflows.find_run(run.id).await?;
                Ok(Some(match stored.map(|run| run.status) {
                    Some(RunStatus::Pending) => RunOutcome::Retry,
                    Some(RunStatus::Succeeded) => RunOutcome::Succeeded,
                    Some(RunStatus::Failed) | None => RunOutcome::Failed,
                }))
            }
            Err(error) => Err(error),
        }
    }

    async fn perform(
        &self,
        action_index: usize,
        action: &WorkflowAction,
        run: &WorkflowRun,
    ) -> Result<ActionProgress, ActionFailure> {
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
                    None => Ok(ActionProgress::Webhook(result.status)),
                    Some(message) => Err(ActionFailure {
                        message,
                        retryable: result.retryable,
                        response_status: result.status,
                    }),
                }
            }
            _ => self
                .documents
                .apply_workflow_document_action(run, action_index as u32)
                .await
                .map(|run_finished| ActionProgress::Document { run_finished })
                .map_err(|error| ActionFailure {
                    retryable: !matches!(
                        error,
                        Error::NotFound { .. } | Error::Validation(_) | Error::Conflict(_)
                    ),
                    message: match error {
                        Error::NotFound {
                            entity_type: "Document",
                            ..
                        } => DOCUMENT_UNAVAILABLE.into(),
                        other => other.to_string(),
                    },
                    response_status: None,
                }),
        }
    }
}

fn advance_to_persisted_progress(
    run: &mut WorkflowRun,
    stored: &WorkflowRun,
    index: usize,
) -> bool {
    run.attempts = run.attempts.max(stored.attempts);
    if stored.completed_actions as usize <= index {
        return false;
    }
    run.completed_actions = stored.completed_actions;
    run.response_status = stored.response_status;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use folio_core::models::{VaultId, WorkflowTrigger};

    #[test]
    fn keeps_the_persisted_attempt_count_at_equal_action_progress() {
        let mut run = WorkflowRun::new(
            VaultId(Id::new_v4()),
            Id::new_v4(),
            WorkflowTrigger::DocumentUploaded,
            None,
            "{}".into(),
        );
        run.completed_actions = 1;
        run.attempts = 2;
        let mut stored = run.clone();
        stored.attempts = 3;

        assert!(!advance_to_persisted_progress(&mut run, &stored, 1));
        assert_eq!(run.attempts, 3);
        assert_eq!(run.completed_actions, 1);
    }
}
