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

use super::{FailOutcome, Job, JobClass, JobStatus, JobType, DEFAULT_MAX_ATTEMPTS};

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
}

macro_rules! job_columns {
    () => {
        "id, status, payload::text AS payload, attempts, max_attempts, error, \
         created_at, updated_at, scheduled_at, started_at, completed_at"
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
             SET status = 'processing', started_at = $1, updated_at = $1, attempts = attempts + 1
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
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err("Failed to dequeue job"))?;

        row.map(TryInto::try_into).transpose()
    }

    #[instrument(skip(self))]
    pub async fn complete(&self, job_id: Uuid) -> Result<()> {
        let now = Utc::now();

        sqlx::query(
            "UPDATE jobs SET status = 'completed', completed_at = $1, updated_at = $1
             WHERE id = $2",
        )
        .bind(now)
        .bind(job_id)
        .execute(&self.pool)
        .await
        .map_err(repo_err("Failed to complete job"))?;

        debug!("Completed job {}", job_id);
        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn fail(&self, job_id: Uuid, error: &str, retry: bool) -> Result<FailOutcome> {
        let now = Utc::now();

        let row: Option<(i32, i32)> =
            sqlx::query_as("SELECT attempts, max_attempts FROM jobs WHERE id = $1")
                .bind(job_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err("Failed to get job"))?;

        let (attempts, max_attempts) = row.unwrap_or((1, 3));

        if retry && attempts < max_attempts {
            let delay_seconds = (2_i64).pow(attempts as u32) * 10; // 20s, 40s, ...
            let retry_at = now + Duration::seconds(delay_seconds);

            sqlx::query(
                "UPDATE jobs SET status = 'pending', error = $1, scheduled_at = $2, updated_at = $3
                 WHERE id = $4",
            )
            .bind(error)
            .bind(retry_at)
            .bind(now)
            .bind(job_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err("Failed to retry job"))?;

            warn!(
                "Job {} failed (attempt {}/{}), retrying in {}s: {}",
                job_id, attempts, max_attempts, delay_seconds, error
            );
            return Ok(FailOutcome::Retrying);
        } else {
            sqlx::query(
                "UPDATE jobs SET status = 'failed', error = $1, completed_at = $2, updated_at = $2
                 WHERE id = $3",
            )
            .bind(error)
            .bind(now)
            .bind(job_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err("Failed to fail job"))?;

            error!(
                "Job {} permanently failed after {} attempts: {}",
                job_id, attempts, error
            );
        }

        Ok(FailOutcome::GaveUp)
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

    pub async fn reset_stuck(&self, minutes: i64) -> Result<i64> {
        let now = Utc::now();
        let cutoff = now - Duration::minutes(minutes);

        let result = sqlx::query(
            "UPDATE jobs SET status = 'pending', updated_at = $1
             WHERE status = 'processing' AND started_at < $2",
        )
        .bind(now)
        .bind(cutoff)
        .execute(&self.pool)
        .await
        .map_err(repo_err("Failed to reset stuck jobs"))?;

        if result.rows_affected() > 0 {
            info!("Reset {} stuck jobs", result.rows_affected());
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
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM jobs
             WHERE payload->'data'->>'document_id' = $1 AND status = 'failed'",
        )
        .bind(document_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err("Failed to check failed jobs"))?;

        Ok(count > 0)
    }
}
