//! PostgreSQL job queue backend
//!
//! Jobs are claimed with `FOR UPDATE SKIP LOCKED`, so several workers (or
//! several Folio instances) can poll the same table without handing out the
//! same job twice.

use chrono::{DateTime, Duration, Utc};
use sqlx::{PgConnection, PgPool};
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use folio_core::error::{Error, Result};

use super::{
    damaged_row_message, payload_problem, FailOutcome, Job, JobClass, JobStatus, JobType,
    DEFAULT_MAX_ATTEMPTS, LEASE_SECONDS, MAX_UNREADABLE_PER_DEQUEUE,
};

/// Insert a pending job; pass a transaction to commit it together with other writes
pub(crate) async fn insert_job(
    conn: &mut PgConnection,
    job_type: JobType,
    scheduled_at: DateTime<Utc>,
    max_attempts: i32,
    trace: Option<&super::JobTraceContext>,
) -> Result<Uuid> {
    job_type.ensure_dispatchable()?;
    let plan =
        super::processing_scope::postgres::prepare_enqueue(conn, &job_type, Uuid::new_v4()).await?;
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
        "INSERT INTO jobs (id, job_type, payload, status, attempts, max_attempts,
                           created_at, updated_at, scheduled_at, origin_vault_id, correlation_id, causation_id, processing_group_id,
                           trace_parent, trace_state)
         VALUES ($1, $2, $3::jsonb, 'pending', 0, $4, $5, $5, $6,
                 $7, $8, $9, $10, $11, $12)",
    )
    .bind(id)
    .bind(type_name)
    .bind(&payload)
    .bind(max_attempts)
    .bind(now)
    .bind(scheduled_at)
    .bind(plan.vault_id)
    .bind(lineage.correlation_id)
    .bind(lineage.causation_id)
    .bind(plan.group_id)
    .bind(trace.map(|trace| trace.traceparent.as_str()))
    .bind(trace.and_then(|trace| trace.tracestate.as_deref()))
    .execute(&mut *conn)
    .await
    .map_err(repo_err("Failed to enqueue job"))?;

    super::processing_scope::postgres::finish_enqueue(conn, &job_type, &plan).await?;

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
    trace_parent: Option<String>,
    trace_state: Option<String>,
}

macro_rules! job_columns {
    () => {
        "id, status, payload::text AS payload, attempts, max_attempts, error, \
         created_at, updated_at, scheduled_at, started_at, completed_at, claim_id, lease_expires_at, \
         trace_parent, trace_state"
    };
}

impl JobRow {
    /// Build the job, or say in plain words which field could not be read
    fn into_job(self) -> std::result::Result<Job, &'static str> {
        let job_type: JobType =
            serde_json::from_str(&self.payload).map_err(|e| payload_problem(&e))?;
        Ok(Job {
            id: self.id,
            job_type,
            status: JobStatus::from(self.status),
            attempts: self.attempts,
            max_attempts: self.max_attempts,
            error: self.error,
            created_at: self.created_at,
            updated_at: self.updated_at,
            scheduled_at: self.scheduled_at,
            started_at: self.started_at,
            completed_at: self.completed_at,
            claim_id: self.claim_id,
            lease_expires_at: self.lease_expires_at,
            trace: super::JobTraceContext::from_columns(self.trace_parent, self.trace_state),
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

    pub(super) fn pool(&self) -> &PgPool {
        &self.pool
    }

    #[instrument(skip(self, trace))]
    pub async fn enqueue(
        &self,
        job_type: JobType,
        trace: Option<super::JobTraceContext>,
    ) -> Result<Uuid> {
        self.enqueue_delayed(job_type, Utc::now(), trace).await
    }

    #[instrument(skip(self, trace))]
    pub async fn enqueue_delayed(
        &self,
        job_type: JobType,
        scheduled_at: DateTime<Utc>,
        trace: Option<super::JobTraceContext>,
    ) -> Result<Uuid> {
        self.enqueue_with_max_attempts(job_type, scheduled_at, DEFAULT_MAX_ATTEMPTS, trace)
            .await
    }

    #[instrument(skip(self, trace))]
    pub async fn enqueue_with_max_attempts(
        &self,
        job_type: JobType,
        scheduled_at: DateTime<Utc>,
        max_attempts: i32,
        trace: Option<super::JobTraceContext>,
    ) -> Result<Uuid> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(repo_err("Failed to begin enqueue"))?;
        let id = insert_job(
            &mut tx,
            job_type,
            scheduled_at,
            max_attempts,
            trace.as_ref(),
        )
        .await?;
        tx.commit()
            .await
            .map_err(repo_err("Failed to commit enqueue"))?;
        Ok(id)
    }

    /// Claim the next job; a row that cannot be read is marked failed and skipped
    pub async fn dequeue(&self, class: Option<JobClass>) -> Result<Option<Job>> {
        for _ in 0..MAX_UNREADABLE_PER_DEQUEUE {
            let Some(row) = self.claim_next(class).await? else {
                return Ok(None);
            };
            let (id, claim_id) = (row.id, row.claim_id);
            match row.into_job() {
                Ok(job) => return Ok(Some(job)),
                Err(problem) => self.quarantine(id, claim_id, problem).await?,
            }
        }
        warn!("Set aside {MAX_UNREADABLE_PER_DEQUEUE} unreadable job rows in one call, pausing");
        Ok(None)
    }

    async fn claim_next(&self, class: Option<JobClass>) -> Result<Option<JobRow>> {
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
        Ok(row)
    }

    /// Take an unreadable row out of circulation, keeping it for an operator to inspect
    async fn quarantine(&self, id: Uuid, claim_id: Option<Uuid>, problem: &str) -> Result<()> {
        warn!(job_id = %id, field = "payload", "Skipping job row with an unreadable field");
        sqlx::query(
            "UPDATE jobs SET status = 'failed', error = $1, updated_at = now(), completed_at = now(), claim_id = NULL, lease_expires_at = NULL WHERE id = $2 AND claim_id = $3",
        )
        .bind(damaged_row_message("payload", problem))
        .bind(id)
        .bind(claim_id)
        .execute(&self.pool)
        .await
        .map_err(repo_err("Failed to quarantine job"))?;
        Ok(())
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
        super::recovery::postgres::fail(&self.pool, job, error, retry, reason).await
    }

    pub async fn context(&self, job_id: Uuid) -> Result<Option<super::JobContext>> {
        let row: Option<(Option<Uuid>, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
            "SELECT origin_vault_id, correlation_id, causation_id FROM jobs WHERE id = $1",
        )
        .bind(job_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err("Failed to read job context"))?;
        let Some((vault, Some(correlation), causation)) = row else {
            return Ok(None);
        };
        Ok(Some(super::JobContext {
            vault_id: vault.map(folio_core::models::VaultId),
            lineage: folio_core::mutation::MutationLineage {
                correlation_id: correlation,
                causation_id: causation,
            },
        }))
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
        super::recovery::postgres::reset_stuck(&self.pool).await
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

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let id = row.id;
                match row.into_job() {
                    Ok(job) => Some(job),
                    Err(_) => {
                        warn!(job_id = %id, field = "payload", "Ignoring job row with an unreadable field");
                        None
                    }
                }
            })
            .collect())
    }

    pub async fn latest_purge_job(&self, document_id: Uuid) -> Result<Option<JobType>> {
        let payload: Option<String> = sqlx::query_scalar("SELECT payload::text FROM jobs WHERE job_type = 'purge_document' AND payload->'data'->>'document_id' = $1 ORDER BY created_at DESC, id DESC LIMIT 1")
            .bind(document_id.to_string()).fetch_optional(&self.pool).await
            .map_err(|error| Error::Repository(error.to_string()))?;
        payload
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| Error::Repository(error.to_string()))
            })
            .transpose()
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
