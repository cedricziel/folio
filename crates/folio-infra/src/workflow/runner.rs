//! Executes queued workflow runs

use std::sync::Arc;

use chrono::Utc;
use tracing::{info, warn};

use folio_core::error::Error;
use folio_core::models::{
    validate_webhook_url, Id, RunStatus, UpdateDocument, VaultId, WorkflowAction, WorkflowRun,
    WorkflowTrigger,
};
use folio_core::ports::{DocumentRepository, WorkflowRepository};

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
        }
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

        let vault = self.workflows.locate(run.workflow_id).await?;
        let workflow = match vault {
            Some(vault) => self.workflows.find_by_id(vault, run.workflow_id).await?,
            None => None,
        };
        let (Some(vault), Some(_workflow)) = (vault, workflow) else {
            self.finish(
                &mut run,
                RunStatus::Failed,
                Some("Workflow no longer exists".into()),
            )
            .await?;
            return Ok(RunOutcome::Failed);
        };

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
            if self.workflows.locate(run.workflow_id).await? != Some(vault) {
                self.finish(
                    &mut run,
                    RunStatus::Failed,
                    Some("Workflow vault is no longer active".into()),
                )
                .await?;
                return Ok(RunOutcome::Failed);
            }
            if run.event != WorkflowTrigger::DocumentDeleted {
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
            match self.perform(vault, index, action, &run).await {
                Ok(response_status) => {
                    run.completed_actions = index as u32 + 1;
                    run.response_status = response_status.or(run.response_status);
                    self.workflows.update_run(&run).await?;
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
                    self.workflows.update_run(&run).await?;
                    return Ok(RunOutcome::Retry);
                }
            }
        }

        self.finish(&mut run, RunStatus::Succeeded, None).await?;
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
        self.finish(&mut run, RunStatus::Failed, Some(error)).await
    }

    async fn finish(
        &self,
        run: &mut WorkflowRun,
        status: RunStatus,
        error: Option<String>,
    ) -> folio_core::Result<()> {
        run.status = status;
        run.last_error = error;
        run.finished_at = Some(Utc::now());
        self.workflows.update_run(run).await
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
