//! SQLite job queue backend

use chrono::{DateTime, Duration, Utc};
use sqlx::{SqliteConnection, SqlitePool};
use tracing::{debug, instrument};
use uuid::Uuid;

use folio_core::error::{Error, Result};

use super::{FailOutcome, Job, JobClass, JobStatus, JobType, DEFAULT_MAX_ATTEMPTS, LEASE_SECONDS};

/// Insert a pending job; pass a transaction to commit it together with other writes
pub(crate) async fn insert_job(
    conn: &mut SqliteConnection,
    job_type: JobType,
    scheduled_at: DateTime<Utc>,
    max_attempts: i32,
) -> Result<Uuid> {
    let plan =
        super::processing_scope::sqlite::prepare_enqueue(conn, &job_type, Uuid::new_v4()).await?;
    if plan.existing {
        return Ok(plan.id);
    }
    let id = plan.id;
    let now = Utc::now();
    let type_name = job_type.type_name();
    let lineage =
        folio_core::mutation::current_lineage().unwrap_or(folio_core::mutation::MutationLineage {
            correlation_id: Uuid::new_v4(),
            causation_id: None,
        });
    let payload = serde_json::to_string(&job_type)
        .map_err(|e| Error::Repository(format!("Failed to serialize job: {}", e)))?;

    sqlx::query(
        r#"
        INSERT INTO jobs (id, job_type, payload, status, attempts, max_attempts, created_at, updated_at, scheduled_at, origin_vault_id, correlation_id, causation_id, processing_group_id)
        VALUES (?, ?, ?, 'pending', 0, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(id.to_string())
    .bind(type_name)
    .bind(&payload)
    .bind(max_attempts)
    .bind(now.to_rfc3339())
    .bind(now.to_rfc3339())
    .bind(scheduled_at.to_rfc3339())
    .bind(plan.vault_id.map(|id| id.to_string()))
    .bind(lineage.correlation_id.to_string())
    .bind(lineage.causation_id.map(|id| id.to_string()))
    .bind(plan.group_id.map(|id| id.to_string()))
    .execute(&mut *conn)
    .await
    .map_err(|e| Error::Repository(format!("Failed to enqueue job: {}", e)))?;

    super::processing_scope::sqlite::finish_enqueue(conn, &job_type, &plan).await?;

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

    pub(super) fn pool(&self) -> &SqlitePool {
        &self.pool
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
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|error| Error::Repository(error.to_string()))?;
        let id = insert_job(&mut tx, job_type, scheduled_at, max_attempts).await?;
        tx.commit()
            .await
            .map_err(|error| Error::Repository(error.to_string()))?;
        Ok(id)
    }

    /// Dequeue the next available job
    #[allow(clippy::type_complexity)]
    pub async fn dequeue(&self, class: Option<JobClass>) -> Result<Option<Job>> {
        let now = Utc::now();
        let class_filter = class.map_or("TRUE", JobClass::condition);

        // Find and claim the next pending job
        let row: Option<(String, String, String, i32, i32, Option<String>, String, String, String, Option<String>, Option<String>, Option<String>, Option<i64>)> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                r#"
                UPDATE jobs
                SET status = 'processing',
                    started_at = ?,
                    updated_at = ?,
                    attempts = attempts + 1,
                    claim_id = ?,
                    lease_expires_at = unixepoch() + ?
                WHERE id = (
                    SELECT id FROM jobs
                    WHERE status = 'pending'
                    AND scheduled_at <= ?
                    AND {class_filter}
                    ORDER BY scheduled_at ASC
                    LIMIT 1
                )
                RETURNING id, job_type, payload, attempts, max_attempts, error, created_at, updated_at, scheduled_at, started_at, completed_at, claim_id, lease_expires_at
                "#,
            )))
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .bind(Uuid::new_v4().to_string())
            .bind(LEASE_SECONDS)
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
                claim_id,
                lease_expires_at,
            )) => {
                let job_type: JobType = serde_json::from_str(&payload)
                    .map_err(|e| Error::Repository(format!("Failed to deserialize job: {}", e)))?;

                Ok(Some(Job {
                    id: Uuid::parse_str(&id).unwrap(),
                    claim_id: claim_id.and_then(|id| Uuid::parse_str(&id).ok()),
                    lease_expires_at,
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

    pub async fn complete(&self, job: &Job) -> Result<bool> {
        let now = Utc::now();
        let result = sqlx::query(
            "UPDATE jobs SET status = 'completed', completed_at = ?, updated_at = ?, claim_id = NULL, lease_expires_at = NULL WHERE id = ? AND claim_id = ? AND status = 'processing' AND lease_expires_at > unixepoch()",
        )
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(job.id.to_string())
        .bind(job.claim_id.map(|id| id.to_string()))
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to complete job: {e}")))?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn renew(&self, job: &Job) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE jobs SET lease_expires_at = unixepoch() + ? WHERE id = ? AND claim_id = ? AND status = 'processing' AND lease_expires_at > unixepoch()",
        )
        .bind(LEASE_SECONDS)
        .bind(job.id.to_string())
        .bind(job.claim_id.map(|id| id.to_string()))
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to renew job lease: {e}")))?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn fail(&self, job: &Job, error: &str, retry: bool) -> Result<FailOutcome> {
        self.fail_with_reason(
            job,
            error,
            retry,
            folio_core::processing_events::ProcessingReason::ProviderFailed,
        )
        .await
    }

    pub async fn fail_with_reason(
        &self,
        job: &Job,
        error: &str,
        retry: bool,
        reason: folio_core::processing_events::ProcessingReason,
    ) -> Result<FailOutcome> {
        super::recovery::sqlite::fail(&self.pool, job, error, retry, reason).await
    }

    pub async fn context(&self, job_id: Uuid) -> Result<Option<super::JobContext>> {
        let row: Option<(Option<String>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT origin_vault_id, correlation_id, causation_id, processing_group_id FROM jobs WHERE id = ?",
        )
        .bind(job_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Repository(e.to_string()))?;
        let Some((vault, Some(correlation), causation, group)) = row else {
            return Ok(None);
        };
        let parse = |id: String| Uuid::parse_str(&id).map_err(|e| Error::Repository(e.to_string()));
        Ok(Some(super::JobContext {
            processing_group_id: group.map(parse).transpose()?,
            vault_id: vault
                .map(parse)
                .transpose()?
                .map(folio_core::models::VaultId),
            lineage: folio_core::mutation::MutationLineage {
                correlation_id: parse(correlation)?,
                causation_id: causation.map(parse).transpose()?,
            },
        }))
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

    pub async fn reset_stuck(&self) -> Result<i64> {
        super::recovery::sqlite::reset_stuck(&self.pool).await
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
            Option<String>,
            Option<i64>,
        )> = sqlx::query_as(
            r#"
            SELECT id, job_type, payload, status, attempts, max_attempts, error,
                   created_at, updated_at, scheduled_at, started_at, completed_at, claim_id, lease_expires_at
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
                claim_id: row.12.and_then(|id| Uuid::parse_str(&id).ok()),
                lease_expires_at: row.13,
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
    pub async fn latest_purge_job(&self, document_id: Uuid) -> Result<Option<JobType>> {
        let payload: Option<String> = sqlx::query_scalar("SELECT payload FROM jobs WHERE job_type = 'purge_document' AND json_extract(payload, '$.data.document_id') = ? ORDER BY created_at DESC, id DESC LIMIT 1")
            .bind(document_id.to_string()).fetch_optional(&self.pool).await
            .map_err(|error| Error::Repository(error.to_string()))?;
        payload
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| Error::Repository(error.to_string()))
            })
            .transpose()
    }

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
