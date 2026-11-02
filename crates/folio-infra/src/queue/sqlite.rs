//! SQLite job queue backend

use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use folio_core::error::{Error, Result};

use super::{FailOutcome, Job, JobClass, JobStatus, JobType, DEFAULT_MAX_ATTEMPTS};

/// Insert a pending job; pass a transaction to commit it together with other writes
pub(crate) async fn insert_job<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    job_type: JobType,
    scheduled_at: DateTime<Utc>,
    max_attempts: i32,
) -> Result<Uuid> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let type_name = job_type.type_name();
    let payload = serde_json::to_string(&job_type)
        .map_err(|e| Error::Repository(format!("Failed to serialize job: {}", e)))?;

    sqlx::query(
        r#"
        INSERT INTO jobs (id, job_type, payload, status, attempts, max_attempts, created_at, updated_at, scheduled_at)
        VALUES (?, ?, ?, 'pending', 0, ?, ?, ?, ?)
        "#,
    )
    .bind(id.to_string())
    .bind(type_name)
    .bind(&payload)
    .bind(max_attempts)
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .bind(scheduled_at.to_rfc3339())
    .execute(executor)
    .await
    .map_err(|e| Error::Repository(format!("Failed to enqueue job: {}", e)))?;

    debug!("Enqueued job {} of type {}", id, type_name);
    Ok(id)
}

/// SQLite-backed job queue
#[derive(Clone)]
pub struct SqliteJobQueue {
    pool: SqlitePool,
}

impl SqliteJobQueue {
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
        self.enqueue_with_max_attempts(job_type, scheduled_at, DEFAULT_MAX_ATTEMPTS)
            .await
    }

    /// Enqueue a job with its own retry budget
    #[instrument(skip(self))]
    pub async fn enqueue_with_max_attempts(
        &self,
        job_type: JobType,
        scheduled_at: DateTime<Utc>,
        max_attempts: i32,
    ) -> Result<Uuid> {
        insert_job(&self.pool, job_type, scheduled_at, max_attempts).await
    }

    /// Dequeue the next available job
    #[allow(clippy::type_complexity)]
    pub async fn dequeue(&self, class: Option<JobClass>) -> Result<Option<Job>> {
        let now = Utc::now();
        let class_filter = class.map_or("TRUE", JobClass::condition);

        // Find and claim the next pending job
        let row: Option<(String, String, String, i32, i32, Option<String>, String, String, String, Option<String>, Option<String>)> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
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
                    AND {class_filter}
                    ORDER BY scheduled_at ASC
                    LIMIT 1
                )
                RETURNING id, job_type, payload, attempts, max_attempts, error, created_at, updated_at, scheduled_at, started_at, completed_at
                "#,
            )))
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
    pub async fn fail(&self, job_id: Uuid, error: &str, retry: bool) -> Result<FailOutcome> {
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
            return Ok(FailOutcome::Retrying);
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

        Ok(FailOutcome::GaveUp)
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

    /// Whether the latest finished job for any processing stage failed
    pub async fn has_failed_jobs_for_document(&self, document_id: Uuid) -> Result<bool> {
        let search_pattern = format!("%\"document_id\":\"{}\"%", document_id);

        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM (
                SELECT status, ROW_NUMBER() OVER (
                    PARTITION BY job_type ORDER BY created_at DESC, completed_at DESC, id DESC
                ) AS position FROM jobs
                WHERE payload LIKE ? AND status IN ('completed', 'failed')
            ) WHERE position = 1 AND status = 'failed'
            "#,
        )
        .bind(&search_pattern)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to check failed jobs: {}", e)))?;

        Ok(row.0 > 0)
    }
}
