//! SQLite-backed job queue for background processing

mod worker;

pub use worker::{Worker, WorkerConfig};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use async_trait::async_trait;

use folio_core::error::{Error, Result};
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
}

impl JobType {
    pub fn type_name(&self) -> &'static str {
        match self {
            JobType::GenerateThumbnail { .. } => "generate_thumbnail",
            JobType::ExtractText { .. } => "extract_text",
            JobType::GenerateEmbedding { .. } => "generate_embedding",
            JobType::ProcessDocument { .. } => "process_document",
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

/// SQLite-backed job queue
#[derive(Clone)]
pub struct JobQueue {
    pool: SqlitePool,
}

impl JobQueue {
    /// Create a new job queue
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Enqueue a new job
    #[instrument(skip(self))]
    pub async fn enqueue(&self, job_type: JobType) -> Result<Uuid> {
        self.enqueue_delayed(job_type, Utc::now()).await
    }

    /// Enqueue a job to run at a specific time
    #[instrument(skip(self))]
    pub async fn enqueue_delayed(
        &self,
        job_type: JobType,
        scheduled_at: DateTime<Utc>,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let type_name = job_type.type_name();
        let payload = serde_json::to_string(&job_type)
            .map_err(|e| Error::Repository(format!("Failed to serialize job: {}", e)))?;

        sqlx::query(
            r#"
            INSERT INTO jobs (id, job_type, payload, status, attempts, max_attempts, created_at, updated_at, scheduled_at)
            VALUES (?, ?, ?, 'pending', 0, 3, ?, ?, ?)
            "#,
        )
        .bind(id.to_string())
        .bind(type_name)
        .bind(&payload)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(scheduled_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to enqueue job: {}", e)))?;

        debug!("Enqueued job {} of type {}", id, type_name);
        Ok(id)
    }

    /// Dequeue the next available job
    #[instrument(skip(self))]
    #[allow(clippy::type_complexity)]
    pub async fn dequeue(&self) -> Result<Option<Job>> {
        let now = Utc::now();

        // Find and claim the next pending job
        let row: Option<(String, String, String, i32, i32, Option<String>, String, String, String, Option<String>, Option<String>)> =
            sqlx::query_as(
                r#"
                UPDATE jobs
                SET status = 'processing',
                    started_at = ?,
                    updated_at = ?,
                    attempts = attempts + 1
                WHERE id = (
                    SELECT id FROM jobs
                    WHERE status = 'pending'
                    AND scheduled_at <= ?
                    ORDER BY scheduled_at ASC
                    LIMIT 1
                )
                RETURNING id, job_type, payload, attempts, max_attempts, error, created_at, updated_at, scheduled_at, started_at, completed_at
                "#,
            )
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::Repository(format!("Failed to dequeue job: {}", e)))?;

        match row {
            Some((
                id,
                _job_type,
                payload,
                attempts,
                max_attempts,
                error,
                created_at,
                updated_at,
                scheduled_at,
                started_at,
                completed_at,
            )) => {
                let job_type: JobType = serde_json::from_str(&payload)
                    .map_err(|e| Error::Repository(format!("Failed to deserialize job: {}", e)))?;

                Ok(Some(Job {
                    id: Uuid::parse_str(&id).unwrap(),
                    job_type,
                    status: JobStatus::Processing,
                    attempts,
                    max_attempts,
                    error,
                    created_at: DateTime::parse_from_rfc3339(&created_at).unwrap().into(),
                    updated_at: DateTime::parse_from_rfc3339(&updated_at).unwrap().into(),
                    scheduled_at: DateTime::parse_from_rfc3339(&scheduled_at).unwrap().into(),
                    started_at: started_at
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(Into::into)),
                    completed_at: completed_at
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(Into::into)),
                }))
            }
            None => Ok(None),
        }
    }

    /// Mark a job as completed
    #[instrument(skip(self))]
    pub async fn complete(&self, job_id: Uuid) -> Result<()> {
        let now = Utc::now();

        sqlx::query(
            r#"
            UPDATE jobs
            SET status = 'completed',
                completed_at = ?,
                updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(job_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to complete job: {}", e)))?;

        debug!("Completed job {}", job_id);
        Ok(())
    }

    /// Mark a job as failed
    #[instrument(skip(self))]
    pub async fn fail(&self, job_id: Uuid, error: &str, retry: bool) -> Result<()> {
        let now = Utc::now();

        // Get current attempt count
        let row: Option<(i32, i32)> =
            sqlx::query_as("SELECT attempts, max_attempts FROM jobs WHERE id = ?")
                .bind(job_id.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| Error::Repository(format!("Failed to get job: {}", e)))?;

        let (attempts, max_attempts) = row.unwrap_or((1, 3));

        if retry && attempts < max_attempts {
            // Schedule retry with exponential backoff
            let delay_seconds = (2_i64).pow(attempts as u32) * 10; // 10s, 20s, 40s, ...
            let retry_at = now + Duration::seconds(delay_seconds);

            sqlx::query(
                r#"
                UPDATE jobs
                SET status = 'pending',
                    error = ?,
                    scheduled_at = ?,
                    updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(error)
            .bind(retry_at.to_rfc3339())
            .bind(now.to_rfc3339())
            .bind(job_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Repository(format!("Failed to retry job: {}", e)))?;

            warn!(
                "Job {} failed (attempt {}/{}), retrying in {}s: {}",
                job_id, attempts, max_attempts, delay_seconds, error
            );
        } else {
            // Mark as permanently failed
            sqlx::query(
                r#"
                UPDATE jobs
                SET status = 'failed',
                    error = ?,
                    completed_at = ?,
                    updated_at = ?
                WHERE id = ?
                "#,
            )
            .bind(error)
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .bind(job_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Repository(format!("Failed to fail job: {}", e)))?;

            error!(
                "Job {} permanently failed after {} attempts: {}",
                job_id, attempts, error
            );
        }

        Ok(())
    }

    /// Get count of pending jobs
    pub async fn pending_count(&self) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM jobs WHERE status = 'pending'")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| Error::Repository(format!("Failed to count jobs: {}", e)))?;

        Ok(row.0)
    }

    /// Clean up old completed jobs (older than specified days)
    pub async fn cleanup(&self, days: i64) -> Result<i64> {
        let cutoff = Utc::now() - Duration::days(days);

        let result =
            sqlx::query("DELETE FROM jobs WHERE status = 'completed' AND completed_at < ?")
                .bind(cutoff.to_rfc3339())
                .execute(&self.pool)
                .await
                .map_err(|e| Error::Repository(format!("Failed to cleanup jobs: {}", e)))?;

        Ok(result.rows_affected() as i64)
    }

    /// Reset stuck processing jobs (older than specified minutes)
    pub async fn reset_stuck(&self, minutes: i64) -> Result<i64> {
        let cutoff = Utc::now() - Duration::minutes(minutes);

        let result = sqlx::query(
            r#"
            UPDATE jobs
            SET status = 'pending',
                updated_at = ?
            WHERE status = 'processing'
            AND started_at < ?
            "#,
        )
        .bind(Utc::now().to_rfc3339())
        .bind(cutoff.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to reset stuck jobs: {}", e)))?;

        if result.rows_affected() > 0 {
            info!("Reset {} stuck jobs", result.rows_affected());
        }

        Ok(result.rows_affected() as i64)
    }

    /// Find active jobs (pending or processing) for a specific document
    #[allow(clippy::type_complexity)]
    pub async fn find_active_jobs_for_document(&self, document_id: Uuid) -> Result<Vec<Job>> {
        // The document_id is stored in the JSON payload, so we search with LIKE
        let search_pattern = format!("%\"document_id\":\"{}\"%", document_id);

        let rows: Vec<(
            String,
            String,
            String,
            String,
            i32,
            i32,
            Option<String>,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
        )> = sqlx::query_as(
            r#"
            SELECT id, job_type, payload, status, attempts, max_attempts, error,
                   created_at, updated_at, scheduled_at, started_at, completed_at
            FROM jobs
            WHERE payload LIKE ?
            AND status IN ('pending', 'processing')
            ORDER BY created_at DESC
            "#,
        )
        .bind(&search_pattern)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to find jobs for document: {}", e)))?;

        let mut jobs = Vec::new();
        for row in rows {
            let job_type: JobType = serde_json::from_str(&row.2)
                .map_err(|e| Error::Repository(format!("Failed to deserialize job: {}", e)))?;

            jobs.push(Job {
                id: Uuid::parse_str(&row.0).unwrap(),
                job_type,
                status: JobStatus::from(row.3),
                attempts: row.4,
                max_attempts: row.5,
                error: row.6,
                created_at: DateTime::parse_from_rfc3339(&row.7).unwrap().into(),
                updated_at: DateTime::parse_from_rfc3339(&row.8).unwrap().into(),
                scheduled_at: DateTime::parse_from_rfc3339(&row.9).unwrap().into(),
                started_at: row
                    .10
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(Into::into)),
                completed_at: row
                    .11
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok().map(Into::into)),
            });
        }

        Ok(jobs)
    }

    /// Check if a document has any failed jobs (for displaying error state)
    pub async fn has_failed_jobs_for_document(&self, document_id: Uuid) -> Result<bool> {
        let search_pattern = format!("%\"document_id\":\"{}\"%", document_id);

        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM jobs
            WHERE payload LIKE ?
            AND status = 'failed'
            "#,
        )
        .bind(&search_pattern)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to check failed jobs: {}", e)))?;

        Ok(row.0 > 0)
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
