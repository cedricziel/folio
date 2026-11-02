//! PostgreSQL job queue backend
//!
//! Jobs are claimed with `FOR UPDATE SKIP LOCKED`, so several workers (or
//! several Folio instances) can poll the same table without handing out the
//! same job twice.

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use folio_core::error::{Error, Result};

use super::{FailOutcome, Job, JobClass, JobStatus, JobType, DEFAULT_MAX_ATTEMPTS, LEASE_SECONDS};

/// Insert a pending job; pass a transaction to commit it together with other writes
pub(crate) async fn insert_job<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Postgres>,
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
        "INSERT INTO jobs (id, job_type, payload, status, attempts, max_attempts,
                           created_at, updated_at, scheduled_at)
         VALUES ($1, $2, $3::jsonb, 'pending', 0, $4, $5, $5, $6)",
    )
    .bind(id)
    .bind(type_name)
    .bind(&payload)
    .bind(max_attempts)
    .bind(now)
    .bind(scheduled_at)
    .execute(executor)
    .await
    .map_err(repo_err("Failed to enqueue job"))?;

    debug!("Enqueued job {} of type {}", id, type_name);
    Ok(id)
}

#[derive(Clone)]
pub struct PostgresJobQueue {
    pool: PgPool,
}

#[derive(sqlx::FromRow)]
struct JobRow {
    id: Uuid,
    status: String,
    payload: String,
    attempts: i32,
    max_attempts: i32,
    error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    scheduled_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    claim_id: Option<Uuid>,
    lease_expires_at: Option<i64>,
}

macro_rules! job_columns {
    () => {
        "id, status, payload::text AS payload, attempts, max_attempts, error, \
         created_at, updated_at, scheduled_at, started_at, completed_at, claim_id, lease_expires_at"
    };
}

impl TryFrom<JobRow> for Job {
    type Error = Error;

    fn try_from(row: JobRow) -> Result<Self> {
        let job_type: JobType = serde_json::from_str(&row.payload)
            .map_err(|e| Error::Repository(format!("Failed to deserialize job: {}", e)))?;
        Ok(Job {
            id: row.id,
            job_type,
            status: JobStatus::from(row.status),
            attempts: row.attempts,
            max_attempts: row.max_attempts,
            error: row.error,
            created_at: row.created_at,
            updated_at: row.updated_at,
            scheduled_at: row.scheduled_at,
            started_at: row.started_at,
            completed_at: row.completed_at,
            claim_id: row.claim_id,
            lease_expires_at: row.lease_expires_at,
        })
    }
}

fn repo_err(context: &'static str) -> impl Fn(sqlx::Error) -> Error {
    move |e| Error::Repository(format!("{context}: {e}"))
}

impl PostgresJobQueue {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[instrument(skip(self))]
    pub async fn enqueue(&self, job_type: JobType) -> Result<Uuid> {
        self.enqueue_delayed(job_type, Utc::now()).await
    }

    #[instrument(skip(self))]
    pub async fn enqueue_delayed(
        &self,
        job_type: JobType,
        scheduled_at: DateTime<Utc>,
    ) -> Result<Uuid> {
        self.enqueue_with_max_attempts(job_type, scheduled_at, DEFAULT_MAX_ATTEMPTS)
            .await
    }

    #[instrument(skip(self))]
    pub async fn enqueue_with_max_attempts(
        &self,
        job_type: JobType,
        scheduled_at: DateTime<Utc>,
        max_attempts: i32,
    ) -> Result<Uuid> {
        insert_job(&self.pool, job_type, scheduled_at, max_attempts).await
    }

    pub async fn dequeue(&self, class: Option<JobClass>) -> Result<Option<Job>> {
        let now = Utc::now();
        let class_filter = class.map_or("TRUE", JobClass::condition);

        let row: Option<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "UPDATE jobs
             SET status = 'processing', started_at = $1, updated_at = $1, attempts = attempts + 1,
                 claim_id = $2, lease_expires_at = floor(extract(epoch FROM clock_timestamp()))::bigint + $3
             WHERE id = (
                 SELECT id FROM jobs
                 WHERE status = 'pending' AND scheduled_at <= $1 AND {class_filter}
                 ORDER BY scheduled_at ASC
                 LIMIT 1
                 FOR UPDATE SKIP LOCKED
             )
             RETURNING {columns}",
            columns = job_columns!()
        )))
        .bind(now)
        .bind(Uuid::new_v4())
        .bind(LEASE_SECONDS)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err("Failed to dequeue job"))?;

        row.map(TryInto::try_into).transpose()
    }

    pub async fn complete(&self, job: &Job) -> Result<bool> {
        let now = Utc::now();
        let result = sqlx::query(
            "UPDATE jobs SET status = 'completed', completed_at = $1, updated_at = $2, claim_id = NULL, lease_expires_at = NULL WHERE id = $3 AND claim_id = $4 AND status = 'processing' AND lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint",
        )
        .bind(now)
        .bind(now)
        .bind(job.id)
        .bind(job.claim_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to complete job: {e}")))?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn renew(&self, job: &Job) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE jobs SET lease_expires_at = floor(extract(epoch FROM clock_timestamp()))::bigint + $1 WHERE id = $2 AND claim_id = $3 AND status = 'processing' AND lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint",
        )
        .bind(LEASE_SECONDS)
        .bind(job.id)
        .bind(job.claim_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to renew job lease: {e}")))?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn fail(&self, job: &Job, error: &str, retry: bool) -> Result<FailOutcome> {
        let now = Utc::now();
        let retrying = retry && job.attempts < job.max_attempts;
        let result = if retrying {
            let delay_seconds = 2_i64.pow(job.attempts as u32) * 10;
            let retry_at = now + Duration::seconds(delay_seconds);
            sqlx::query(
                "UPDATE jobs SET status = 'pending', error = $1, scheduled_at = $2, updated_at = $3, claim_id = NULL, lease_expires_at = NULL WHERE id = $4 AND claim_id = $5 AND status = 'processing' AND lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint",
            )
            .bind(error)
            .bind(retry_at)
            .bind(now)
            .bind(job.id)
            .bind(job.claim_id)
            .execute(&self.pool)
            .await
        } else {
            sqlx::query(
                "UPDATE jobs SET status = 'failed', error = $1, completed_at = $2, updated_at = $3, claim_id = NULL, lease_expires_at = NULL WHERE id = $4 AND claim_id = $5 AND status = 'processing' AND lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint",
            )
            .bind(error)
            .bind(now)
            .bind(now)
            .bind(job.id)
            .bind(job.claim_id)
            .execute(&self.pool)
            .await
        }
        .map_err(|e| Error::Repository(format!("Failed to fail job: {e}")))?;
        if result.rows_affected() == 0 {
            return Ok(FailOutcome::LostLease);
        }
        Ok(if retrying {
            warn!(job.id = %job.id, job.attempt = job.attempts, %error, "Job failed; scheduled retry");
            FailOutcome::Retrying
        } else {
            error!(job.id = %job.id, job.attempt = job.attempts, %error, "Job permanently failed");
            FailOutcome::GaveUp
        })
    }

    pub async fn pending_count(&self) -> Result<i64> {
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status = 'pending'")
            .fetch_one(&self.pool)
            .await
            .map_err(repo_err("Failed to count jobs"))
    }

    pub async fn cleanup(&self, days: i64) -> Result<i64> {
        let cutoff = Utc::now() - Duration::days(days);

        let result =
            sqlx::query("DELETE FROM jobs WHERE status = 'completed' AND completed_at < $1")
                .bind(cutoff)
                .execute(&self.pool)
                .await
                .map_err(repo_err("Failed to cleanup jobs"))?;

        Ok(result.rows_affected() as i64)
    }

    pub async fn reset_stuck(&self) -> Result<i64> {
        let now = Utc::now();
        let result = sqlx::query(
            "UPDATE jobs SET status = 'pending', updated_at = $1, claim_id = NULL, lease_expires_at = NULL WHERE status = 'processing' AND (lease_expires_at IS NULL OR lease_expires_at <= floor(extract(epoch FROM clock_timestamp()))::bigint)",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to recover expired jobs: {e}")))?;
        if result.rows_affected() > 0 {
            info!("Recovered {} expired jobs", result.rows_affected());
        }
        Ok(result.rows_affected() as i64)
    }

    pub async fn find_active_jobs_for_document(&self, document_id: Uuid) -> Result<Vec<Job>> {
        let rows: Vec<JobRow> = sqlx::query_as(concat!(
            "SELECT ",
            job_columns!(),
            " FROM jobs
             WHERE payload->'data'->>'document_id' = $1
               AND status IN ('pending', 'processing')
             ORDER BY created_at DESC"
        ))
        .bind(document_id.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err("Failed to find jobs for document"))?;

        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn has_failed_jobs_for_document(&self, document_id: Uuid) -> Result<bool> {
        let failed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM (
                SELECT status, ROW_NUMBER() OVER (
                    PARTITION BY job_type ORDER BY created_at DESC, completed_at DESC, id DESC
                ) AS position FROM jobs
                WHERE payload->'data'->>'document_id' = $1 AND status IN ('completed', 'failed')
            ) AS latest WHERE position = 1 AND status = 'failed')",
        )
        .bind(document_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err("Failed to check failed jobs"))?;

        Ok(failed)
    }
}
