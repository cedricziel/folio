//! Job queue for background processing (SQLite or PostgreSQL)

mod postgres;
mod sqlite;
mod worker;

use postgres::PostgresJobQueue;
use sqlite::SqliteJobQueue;
pub use worker::{Worker, WorkerConfig};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, SqlitePool};
use uuid::Uuid;

use async_trait::async_trait;

use folio_core::error::Result;
use folio_core::ports::{ProcessingQueue, ProcessingStatus};

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
    /// Extract text using OCR
    ExtractText { document_id: Uuid },
    /// Generate embedding for search
    GenerateEmbedding { document_id: Uuid },
    /// Process a newly uploaded document (all steps)
    ProcessDocument { document_id: Uuid },
    /// Execute the actions of a workflow run
    ExecuteWorkflowRun { run_id: Uuid },
}

impl JobType {
    pub fn type_name(&self) -> &'static str {
        match self {
            JobType::GenerateThumbnail { .. } => "generate_thumbnail",
            JobType::ExtractText { .. } => "extract_text",
            JobType::GenerateEmbedding { .. } => "generate_embedding",
            JobType::ProcessDocument { .. } => "process_document",
            JobType::ExecuteWorkflowRun { .. } => "execute_workflow_run",
        }
    }
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
}

/// Retry budget of a job unless the caller sets one
pub(crate) const DEFAULT_MAX_ATTEMPTS: i32 = 3;

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
        dispatch!(self, enqueue(job_type))
    }

    /// Enqueue a job to run at a specific time
    pub async fn enqueue_delayed(
        &self,
        job_type: JobType,
        scheduled_at: DateTime<Utc>,
    ) -> Result<Uuid> {
        dispatch!(self, enqueue_delayed(job_type, scheduled_at))
    }

    /// Enqueue a job with its own retry budget
    pub async fn enqueue_with_max_attempts(
        &self,
        job_type: JobType,
        scheduled_at: DateTime<Utc>,
        max_attempts: i32,
    ) -> Result<Uuid> {
        dispatch!(
            self,
            enqueue_with_max_attempts(job_type, scheduled_at, max_attempts)
        )
    }

    /// Claim the next available job
    pub async fn dequeue(&self) -> Result<Option<Job>> {
        dispatch!(self, dequeue())
    }

    /// Mark a job as completed
    pub async fn complete(&self, job_id: Uuid) -> Result<()> {
        dispatch!(self, complete(job_id))
    }

    /// Mark a job as failed, rescheduling it when `retry` is set and attempts remain
    pub async fn fail(&self, job_id: Uuid, error: &str, retry: bool) -> Result<()> {
        dispatch!(self, fail(job_id, error, retry))
    }

    /// Get count of pending jobs
    pub async fn pending_count(&self) -> Result<i64> {
        dispatch!(self, pending_count())
    }

    /// Delete completed jobs older than the given number of days
    pub async fn cleanup(&self, days: i64) -> Result<i64> {
        dispatch!(self, cleanup(days))
    }

    /// Reset processing jobs that started more than the given minutes ago
    pub async fn reset_stuck(&self, minutes: i64) -> Result<i64> {
        dispatch!(self, reset_stuck(minutes))
    }

    /// Find active jobs (pending or processing) for a specific document
    pub async fn find_active_jobs_for_document(&self, document_id: Uuid) -> Result<Vec<Job>> {
        dispatch!(self, find_active_jobs_for_document(document_id))
    }

    /// Check if a document has any failed jobs
    pub async fn has_failed_jobs_for_document(&self, document_id: Uuid) -> Result<bool> {
        dispatch!(self, has_failed_jobs_for_document(document_id))
    }
}

#[async_trait]
impl ProcessingQueue for JobQueue {
    async fn enqueue_processing(&self, document_id: Uuid) -> Result<()> {
        self.enqueue(JobType::ProcessDocument { document_id })
            .await
            .map(|_| ())
    }

    async fn processing_status(&self, document_id: Uuid) -> Result<ProcessingStatus> {
        let active = self.find_active_jobs_for_document(document_id).await?;
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
