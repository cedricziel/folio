use std::future::Future;

use folio_core::{models::VaultId, Error, Result};
use uuid::Uuid;

use super::{Job, JobQueue, JobType};

pub(super) mod postgres;
pub(super) mod sqlite;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy)]
pub(super) struct ProcessingScope {
    pub group_id: Uuid,
    pub document_id: Uuid,
    pub vault_id: VaultId,
    pub revision_id: Option<Uuid>,
    pub parent_job_id: Uuid,
    pub claim_id: Uuid,
    pub queue_attempt: i32,
}

tokio::task_local! {
    static PROCESSING_SCOPE: Option<ProcessingScope>;
}

pub(super) async fn with_scope<T>(
    scope: Option<ProcessingScope>,
    work: impl Future<Output = T>,
) -> T {
    PROCESSING_SCOPE.scope(scope, work).await
}

fn child_scope(job: &JobType) -> Result<Option<ProcessingScope>> {
    if !matches!(
        job,
        JobType::ExtractText { .. }
            | JobType::ExtractRevisionText { .. }
            | JobType::GenerateEmbedding { .. }
            | JobType::GenerateRevisionEmbedding { .. }
    ) {
        return Ok(None);
    }
    let scope = PROCESSING_SCOPE
        .try_with(|scope| *scope)
        .ok()
        .flatten()
        .filter(|scope| Some(scope.document_id) == job.document_id());
    if scope.is_some_and(|scope| scope.revision_id != job.revision_id()) {
        return Err(stale_parent());
    }
    Ok(scope)
}

fn stages(job: &JobType) -> &'static [&'static str] {
    match job {
        JobType::ProcessDocument { .. } | JobType::ProcessDocumentRevision { .. } => {
            &["thumbnail", "ocr", "embedding"]
        }
        JobType::ExtractText { .. } | JobType::ExtractRevisionText { .. } => &["ocr", "embedding"],
        JobType::GenerateThumbnail { .. } | JobType::GenerateRevisionThumbnail { .. } => {
            &["thumbnail"]
        }
        JobType::GenerateEmbedding { .. } | JobType::GenerateRevisionEmbedding { .. } => {
            &["embedding"]
        }
        JobType::PurgeDocument { .. } | JobType::ExecuteWorkflowRun { .. } => &[],
    }
}

#[derive(sqlx::FromRow)]
struct DocumentScope {
    vault_id: String,
    unavailable: bool,
    revision_id: Option<String>,
    legacy_current: bool,
}

impl DocumentScope {
    fn accepts_revision(&self, revision_id: Option<Uuid>) -> bool {
        match revision_id {
            Some(id) => self.revision_id.as_deref() == Some(id.to_string().as_str()),
            None => self.legacy_current,
        }
    }
}

fn validate_enqueue(document: Option<&DocumentScope>, job: &JobType) -> Result<()> {
    if stages(job).is_empty() {
        return Ok(());
    }
    let revision = job.revision_id();
    match document.filter(|document| !document.unavailable) {
        Some(document) if document.accepts_revision(revision) => Ok(()),
        None if revision.is_none() => Ok(()),
        _ => Err(Error::Conflict(
            "Processing revision is no longer current".into(),
        )),
    }
}

fn assigned_stage(job: &JobType) -> Option<&'static str> {
    stages(job).first().copied()
}

pub(super) struct EnqueuePlan {
    pub(super) id: Uuid,
    pub(super) vault_id: Option<Uuid>,
    pub(super) group_id: Option<Uuid>,
    pub(super) existing: bool,
    pub(super) child_stage: Option<&'static str>,
}

pub(super) enum ClaimPreparation {
    Ready(Option<ProcessingScope>),
    Obsolete,
}

pub(super) async fn prepare_claim(queue: &JobQueue, job: &Job) -> Result<ClaimPreparation> {
    job.job_type.ensure_dispatchable()?;
    if stages(&job.job_type).is_empty() {
        return Ok(ClaimPreparation::Ready(None));
    }
    match queue {
        JobQueue::Sqlite(queue) => sqlite::prepare_claim(queue.pool(), job).await,
        JobQueue::Postgres(queue) => postgres::prepare_claim(queue.pool(), job).await,
    }
}

fn repo_err(error: impl std::fmt::Display) -> Error {
    Error::Repository(error.to_string())
}

const STALE_PARENT: &str = "processing parent claim or group is no longer current";

pub(super) fn is_stale_parent(error: &Error) -> bool {
    matches!(error, Error::Conflict(reason) if reason == STALE_PARENT)
}

fn stale_parent() -> Error {
    Error::Conflict(STALE_PARENT.into())
}

fn payload_type(job: &JobType) -> Result<String> {
    serde_json::to_value(job)
        .map_err(repo_err)?
        .get("type")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| repo_err("missing processing job type"))
}
