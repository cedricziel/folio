//! Job queue for background processing (SQLite or PostgreSQL)

mod archive_jobs;
mod postgres;
mod processing_scope;
mod recovery;
mod sqlite;
pub mod stages;
mod trace_context;
mod worker;

pub use archive_jobs::ArchiveService;
use postgres::PostgresJobQueue;
pub(crate) use postgres::{
    insert_current_processing as insert_current_processing_postgres,
    insert_current_thumbnail as insert_current_thumbnail_postgres,
    insert_job as insert_job_postgres,
};
use processing_scope::ProcessingRequest;
use sqlite::SqliteJobQueue;
pub(crate) use sqlite::{
    insert_current_processing as insert_current_processing_sqlite,
    insert_current_thumbnail as insert_current_thumbnail_sqlite, insert_job as insert_job_sqlite,
};
pub use trace_context::JobTraceContext;
pub use worker::{Worker, WorkerConfig};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, SqlitePool};
use uuid::Uuid;

use async_trait::async_trait;

use folio_core::error::Result;
use folio_core::events::EventOrigin;
use folio_core::models::VaultId;
use folio_core::ports::{ProcessingQueue, ProcessingStatus};

pub const REVISION_DISPATCH_ENABLED: bool = true;

/// Job status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Processing,
    Completed,
    Failed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Pending => write!(f, "pending"),
            JobStatus::Processing => write!(f, "processing"),
            JobStatus::Completed => write!(f, "completed"),
            JobStatus::Failed => write!(f, "failed"),
        }
    }
}

impl From<String> for JobStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "pending" => JobStatus::Pending,
            "processing" => JobStatus::Processing,
            "completed" => JobStatus::Completed,
            "failed" => JobStatus::Failed,
            _ => JobStatus::Pending,
        }
    }
}

/// Job types for different background tasks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum JobType {
    /// Generate thumbnail for a document
    GenerateThumbnail { document_id: Uuid },
    /// Extract text using OCR; `force` reruns OCR even when the document already has text
    ExtractText {
        document_id: Uuid,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        force: bool,
    },
    /// Generate embedding for search; its `processed` event carries `origin`
    GenerateEmbedding {
        document_id: Uuid,
        #[serde(default, skip_serializing_if = "is_user")]
        origin: EventOrigin,
    },
    /// Process a newly uploaded document (all steps); `force` is passed on to OCR
    ProcessDocument {
        document_id: Uuid,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        force: bool,
    },
    ProcessDocumentRevision {
        document_id: Uuid,
        revision_id: Uuid,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        force: bool,
    },
    GenerateRevisionThumbnail {
        document_id: Uuid,
        revision_id: Uuid,
    },
    ExtractRevisionText {
        document_id: Uuid,
        revision_id: Uuid,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        force: bool,
    },
    GenerateRevisionEmbedding {
        document_id: Uuid,
        revision_id: Uuid,
        #[serde(default, skip_serializing_if = "is_user")]
        origin: EventOrigin,
    },
    GenerateArchive {
        vault_id: VaultId,
        document_id: Uuid,
        source_revision_id: Uuid,
        attempt_id: Uuid,
    },
    /// Remove objects and metadata after an explicit permanent-deletion request.
    PurgeDocument {
        document_id: Uuid,
        #[serde(default)]
        correlation_id: Option<Uuid>,
        #[serde(default)]
        causation_id: Option<Uuid>,
    },
    /// Execute the actions of a workflow run
    ExecuteWorkflowRun { run_id: Uuid },
}

fn is_user(origin: &EventOrigin) -> bool {
    *origin == EventOrigin::User
}

impl JobType {
    pub fn generate_embedding(document_id: Uuid) -> Self {
        JobType::GenerateEmbedding {
            document_id,
            origin: EventOrigin::User,
        }
    }

    fn embedding_for_revision(document_id: Uuid, revision_id: Option<Uuid>) -> Self {
        match revision_id {
            Some(revision_id) => Self::GenerateRevisionEmbedding {
                document_id,
                revision_id,
                origin: EventOrigin::User,
            },
            None => Self::generate_embedding(document_id),
        }
    }

    pub fn process_document(document_id: Uuid) -> Self {
        JobType::ProcessDocument {
            document_id,
            force: false,
        }
    }

    pub fn extract_text(document_id: Uuid) -> Self {
        JobType::ExtractText {
            document_id,
            force: false,
        }
    }

    /// The document this job works on, if it is a document job.
    pub fn document_id(&self) -> Option<Uuid> {
        match self {
            JobType::GenerateThumbnail { document_id }
            | JobType::ExtractText { document_id, .. }
            | JobType::GenerateEmbedding { document_id, .. }
            | JobType::GenerateArchive { document_id, .. }
            | JobType::PurgeDocument { document_id, .. }
            | JobType::ProcessDocument { document_id, .. }
            | JobType::ProcessDocumentRevision { document_id, .. }
            | JobType::GenerateRevisionThumbnail { document_id, .. }
            | JobType::ExtractRevisionText { document_id, .. }
            | JobType::GenerateRevisionEmbedding { document_id, .. } => Some(*document_id),
            JobType::ExecuteWorkflowRun { .. } => None,
        }
    }

    fn new_queue_id(&self) -> Uuid {
        match self {
            Self::GenerateArchive { attempt_id, .. } => *attempt_id,
            _ => Uuid::new_v4(),
        }
    }

    pub fn revision_id(&self) -> Option<Uuid> {
        match self {
            JobType::GenerateArchive {
                source_revision_id, ..
            } => Some(*source_revision_id),
            JobType::ProcessDocumentRevision { revision_id, .. }
            | JobType::GenerateRevisionThumbnail { revision_id, .. }
            | JobType::ExtractRevisionText { revision_id, .. }
            | JobType::GenerateRevisionEmbedding { revision_id, .. } => Some(*revision_id),
            _ => None,
        }
    }

    fn ensure_dispatchable(&self) -> Result<()> {
        if self.revision_id().is_some()
            && !matches!(self, Self::GenerateArchive { .. })
            && !REVISION_DISPATCH_ENABLED
        {
            return Err(folio_core::Error::Conflict(
                "File revision processing is not yet enabled".into(),
            ));
        }
        Ok(())
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            JobType::GenerateThumbnail { .. } => "generate_thumbnail",
            JobType::ExtractText { .. } => "extract_text",
            JobType::GenerateEmbedding { .. } => "generate_embedding",
            JobType::ProcessDocument { .. } => "process_document",
            JobType::ProcessDocumentRevision { .. } => "process_document_revision",
            JobType::GenerateRevisionThumbnail { .. } => "generate_revision_thumbnail",
            JobType::ExtractRevisionText { .. } => "extract_revision_text",
            JobType::GenerateRevisionEmbedding { .. } => "generate_revision_embedding",
            JobType::GenerateArchive { .. } => "generate_archive",
            JobType::PurgeDocument { .. } => "purge_document",
            JobType::ExecuteWorkflowRun { .. } => "execute_workflow_run",
        }
    }
}

/// Which jobs a worker loop takes
///
/// Webhook delivery can stall for a long time on a dead target, so it has its
/// own class and its own loop instead of queueing behind OCR, and vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobClass {
    /// OCR, thumbnails, embeddings and purge
    Processing,
    /// Workflow runs
    Workflow,
    /// PDF/A conversion
    Archive,
}

impl JobClass {
    /// SQL condition on `job_type` selecting this class
    pub(crate) fn condition(self) -> &'static str {
        match self {
            Self::Processing => "job_type NOT IN ('execute_workflow_run', 'generate_archive')",
            Self::Archive => "job_type = 'generate_archive'",
            Self::Workflow => "job_type = 'execute_workflow_run'",
        }
    }
}

/// What happened to a job that was reported as failed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailOutcome {
    /// Rescheduled with backoff
    Retrying,
    /// Out of attempts or not retryable; it will not run again
    GaveUp,
    /// The claim expired or another worker owns the job.
    LostLease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobContext {
    pub vault_id: Option<folio_core::models::VaultId>,
    pub lineage: folio_core::mutation::MutationLineage,
}

/// A job in the queue
#[derive(Debug, Clone)]
pub struct Job {
    pub id: Uuid,
    pub job_type: JobType,
    pub status: JobStatus,
    pub attempts: i32,
    pub max_attempts: i32,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub scheduled_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub claim_id: Option<Uuid>,
    pub lease_expires_at: Option<i64>,
    /// Trace context the job was queued under; None for untraced and older jobs
    pub trace: Option<JobTraceContext>,
}

/// Retry budget of a job unless the caller sets one
pub(crate) const DEFAULT_MAX_ATTEMPTS: i32 = 3;
pub(crate) const LEASE_SECONDS: i64 = 300;
/// Most unreadable rows one dequeue call sets aside before handing control back to the worker
pub(crate) const MAX_UNREADABLE_PER_DEQUEUE: usize = 100;

/// Plain-words reason a stored payload cannot be read; never quotes the payload
pub(crate) fn payload_problem(error: &serde_json::Error) -> &'static str {
    use serde_json::error::Category;
    match error.classify() {
        Category::Data if error.to_string().starts_with("unknown variant") => "unknown job type",
        Category::Data => "wrong shape",
        _ => "not valid JSON",
    }
}

/// The error stored on a job row that was set aside because a field could not be read
pub(crate) fn damaged_row_message(field: &str, problem: &str) -> String {
    format!("Job row is damaged: its {field} could not be read ({problem})")
}
pub(crate) const HEARTBEAT_SECONDS: u64 = 60;

/// Job queue backed by the configured database.
#[derive(Clone)]
pub enum JobQueue {
    Sqlite(SqliteJobQueue),
    Postgres(PostgresJobQueue),
}

macro_rules! dispatch {
    ($self:ident, $method:ident($($arg:expr),*)) => {
        match $self {
            JobQueue::Sqlite(q) => q.$method($($arg),*).await,
            JobQueue::Postgres(q) => q.$method($($arg),*).await,
        }
    };
}

impl JobQueue {
    pub fn sqlite(pool: SqlitePool) -> Self {
        Self::Sqlite(SqliteJobQueue::new(pool))
    }

    pub fn postgres(pool: PgPool) -> Self {
        Self::Postgres(PostgresJobQueue::new(pool))
    }

    /// Enqueue a new job
    pub async fn enqueue(&self, job_type: JobType) -> Result<Uuid> {
        let trace = JobTraceContext::current();
        dispatch!(self, enqueue(job_type, trace))
    }

    /// Enqueue a job without linking it to the current trace, for bulk enqueues
    /// that would otherwise put every job into one trace
    pub async fn enqueue_untraced(&self, job_type: JobType) -> Result<Uuid> {
        dispatch!(self, enqueue(job_type, None))
    }

    /// Enqueue a job to run at a specific time
    pub async fn enqueue_delayed(
        &self,
        job_type: JobType,
        scheduled_at: DateTime<Utc>,
    ) -> Result<Uuid> {
        let trace = JobTraceContext::current();
        dispatch!(self, enqueue_delayed(job_type, scheduled_at, trace))
    }

    /// Enqueue a job with its own retry budget
    pub async fn enqueue_with_max_attempts(
        &self,
        job_type: JobType,
        scheduled_at: DateTime<Utc>,
        max_attempts: i32,
    ) -> Result<Uuid> {
        let trace = JobTraceContext::current();
        dispatch!(
            self,
            enqueue_with_max_attempts(job_type, scheduled_at, max_attempts, trace)
        )
    }

    /// Schedule new work for the current original in an authorized vault.
    pub async fn enqueue_current_processing(
        &self,
        vault: folio_core::models::VaultId,
        document_id: Uuid,
        force: bool,
    ) -> Result<Uuid> {
        self.enqueue_requested_processing(vault, document_id, ProcessingRequest::Current { force })
            .await
    }

    async fn enqueue_requested_processing(
        &self,
        vault: folio_core::models::VaultId,
        document_id: Uuid,
        request: ProcessingRequest,
    ) -> Result<Uuid> {
        let trace = JobTraceContext::current();
        dispatch!(
            self,
            enqueue_requested_processing(vault, document_id, request, trace)
        )
    }

    pub(super) async fn repair_current_processing(
        &self,
        vault: folio_core::models::VaultId,
        document_id: Uuid,
        stage: folio_core::models::ProcessingStage,
    ) -> Result<bool> {
        dispatch!(self, repair_current_processing(vault, document_id, stage))
    }

    /// Re-embed documents whose current OCR text is ready, without inheriting the settings trace.
    pub async fn reembed_documents(&self) -> Result<usize> {
        dispatch!(self, reembed_documents())
    }

    /// Claim the next available job of any class
    pub async fn dequeue(&self) -> Result<Option<Job>> {
        dispatch!(self, dequeue(None))
    }

    /// Claim the next available job of one class
    pub async fn dequeue_class(&self, class: JobClass) -> Result<Option<Job>> {
        dispatch!(self, dequeue(Some(class)))
    }

    /// Complete a live claim; return false when ownership has expired or changed.
    pub async fn complete(&self, job: &Job) -> Result<bool> {
        dispatch!(self, complete(job))
    }

    /// Start a seeded embedding or thumbnail attempt under its queue claim.
    pub async fn start_derived(
        &self,
        job: &Job,
        stage: folio_core::processing_events::ProcessingEventStage,
    ) -> Result<Option<stages::StageFence>> {
        match self {
            Self::Sqlite(queue) => stages::sqlite::start_derived(queue.pool(), job, stage).await,
            Self::Postgres(queue) => {
                stages::postgres::start_derived(queue.pool(), job, stage).await
            }
        }
    }

    /// Commit embedding chunks and their terminal event atomically.
    pub async fn complete_embedding(
        &self,
        fence: &stages::StageFence,
        expected_content: &str,
        model: &str,
        chunks: &[folio_core::ports::EmbeddedChunk],
    ) -> Result<bool> {
        match self {
            Self::Sqlite(queue) => {
                stages::sqlite::complete_embedding(
                    queue.pool(),
                    fence,
                    expected_content,
                    model,
                    chunks,
                )
                .await
            }
            Self::Postgres(queue) => {
                stages::postgres::complete_embedding(
                    queue.pool(),
                    fence,
                    expected_content,
                    model,
                    chunks,
                )
                .await
            }
        }
    }

    /// Skip a derived stage with an allowlisted reason.
    pub async fn skip_derived(
        &self,
        fence: &stages::StageFence,
        reason: folio_core::processing_events::ProcessingReason,
    ) -> Result<bool> {
        match self {
            Self::Sqlite(queue) => stages::sqlite::skip_derived(queue.pool(), fence, reason).await,
            Self::Postgres(queue) => {
                stages::postgres::skip_derived(queue.pool(), fence, reason).await
            }
        }
    }

    /// Publish thumbnail references and their terminal event atomically.
    pub async fn complete_thumbnail(
        &self,
        fence: &stages::StageFence,
        page_count: Option<u32>,
    ) -> Result<bool> {
        match self {
            Self::Sqlite(queue) => {
                stages::sqlite::complete_thumbnail(queue.pool(), fence, page_count).await
            }
            Self::Postgres(queue) => {
                stages::postgres::complete_thumbnail(queue.pool(), fence, page_count).await
            }
        }
    }

    /// Mark a job as failed, rescheduling it when `retry` is set and attempts remain
    pub async fn fail(&self, job: &Job, error: &str, retry: bool) -> Result<FailOutcome> {
        dispatch!(self, fail(job, error, retry))
    }

    pub async fn fail_with_reason(
        &self,
        job: &Job,
        error: &str,
        retry: bool,
        reason: folio_core::processing_events::ProcessingReason,
    ) -> Result<FailOutcome> {
        dispatch!(self, fail_with_reason(job, error, retry, reason))
    }

    pub async fn start_ocr(&self, job: &Job) -> Result<Option<stages::OcrStageFence>> {
        match self {
            Self::Sqlite(queue) => stages::sqlite::start_ocr(queue.pool(), job).await,
            Self::Postgres(queue) => stages::postgres::start_ocr(queue.pool(), job).await,
        }
    }

    /// `embed` false skips embedding with `provider_disabled` instead of queueing it.
    pub async fn complete_ocr(
        &self,
        fence: &stages::OcrStageFence,
        content: &str,
        embed: bool,
    ) -> Result<bool> {
        match self {
            Self::Sqlite(queue) => {
                stages::sqlite::complete_ocr(queue.pool(), fence, content, embed).await
            }
            Self::Postgres(queue) => {
                stages::postgres::complete_ocr(queue.pool(), fence, content, embed).await
            }
        }
    }

    pub async fn skip_ocr(&self, fence: &stages::OcrStageFence, embed: bool) -> Result<bool> {
        match self {
            Self::Sqlite(queue) => stages::sqlite::skip_ocr(queue.pool(), fence, embed).await,
            Self::Postgres(queue) => stages::postgres::skip_ocr(queue.pool(), fence, embed).await,
        }
    }

    /// Renew only the current, unexpired claim. An expired claim cannot be revived.
    pub async fn renew(&self, job: &Job) -> Result<bool> {
        dispatch!(self, renew(job))
    }

    /// Returns the enqueue-time scope, or None for jobs created before context persistence.
    pub async fn context(&self, job_id: Uuid) -> Result<Option<JobContext>> {
        dispatch!(self, context(job_id))
    }

    /// Get count of pending jobs
    pub async fn pending_count(&self) -> Result<i64> {
        dispatch!(self, pending_count())
    }

    /// Delete completed jobs older than the given number of days
    pub async fn cleanup(&self, days: i64) -> Result<i64> {
        dispatch!(self, cleanup(days))
    }

    /// Recover processing jobs whose lease expired, including pre-lease jobs.
    pub async fn reset_stuck(&self) -> Result<i64> {
        dispatch!(self, reset_stuck())
    }

    /// Find active jobs (pending or processing) for a specific document
    pub async fn find_active_jobs_for_document(&self, document_id: Uuid) -> Result<Vec<Job>> {
        dispatch!(self, find_active_jobs_for_document(document_id))
    }

    pub async fn latest_purge_job(&self, document_id: Uuid) -> Result<Option<JobType>> {
        dispatch!(self, latest_purge_job(document_id))
    }

    /// Whether the latest finished job for any processing stage failed
    pub async fn has_failed_jobs_for_document(&self, document_id: Uuid) -> Result<bool> {
        dispatch!(self, has_failed_jobs_for_document(document_id))
    }
}

#[async_trait]
impl ProcessingQueue for JobQueue {
    async fn enqueue_processing(
        &self,
        vault: folio_core::models::VaultId,
        document_id: Uuid,
    ) -> Result<()> {
        self.enqueue_requested_processing(vault, document_id, ProcessingRequest::InitialUpload)
            .await
            .map(|_| ())
    }

    async fn processing_status(&self, document_id: Uuid) -> Result<ProcessingStatus> {
        let mut active = self.find_active_jobs_for_document(document_id).await?;
        active.retain(|job| !matches!(job.job_type, JobType::GenerateArchive { .. }));
        if active.iter().any(|j| j.status == JobStatus::Processing) {
            Ok(ProcessingStatus::Processing)
        } else if !active.is_empty() {
            Ok(ProcessingStatus::Pending)
        } else if self.has_failed_jobs_for_document(document_id).await? {
            Ok(ProcessingStatus::Failed)
        } else {
            Ok(ProcessingStatus::Completed)
        }
    }
}
