//! SQLite job queue backend

use chrono::{DateTime, Duration, Utc};
use sqlx::{SqliteConnection, SqlitePool};
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use folio_core::error::{Error, Result};

use super::{
    damaged_row_message, payload_problem, FailOutcome, Job, JobClass, JobStatus, JobType,
    DEFAULT_MAX_ATTEMPTS, LEASE_SECONDS, MAX_UNREADABLE_PER_DEQUEUE,
};

/// Insert a pending job; pass a transaction to commit it together with other writes
pub(crate) async fn insert_job(
    conn: &mut SqliteConnection,
    job_type: JobType,
    scheduled_at: DateTime<Utc>,
    max_attempts: i32,
    trace: Option<&super::JobTraceContext>,
) -> Result<Uuid> {
    job_type.ensure_dispatchable()?;
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
        INSERT INTO jobs (id, job_type, payload, status, attempts, max_attempts, created_at, updated_at, scheduled_at, origin_vault_id, correlation_id, causation_id, processing_group_id, trace_parent, trace_state)
        VALUES (?, ?, ?, 'pending', 0, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
    .bind(trace.map(|trace| trace.traceparent.as_str()))
    .bind(trace.and_then(|trace| trace.tracestate.as_deref()))
    .execute(&mut *conn)
    .await
    .map_err(|e| Error::Repository(format!("Failed to enqueue job: {}", e)))?;

    super::processing_scope::sqlite::finish_enqueue(conn, &job_type, &plan).await?;

    debug!("Enqueued job {} of type {}", id, type_name);
    Ok(id)
}

/// A job row as returned by the dequeue claim
type ClaimedRow = (
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
    Option<String>,
    Option<String>,
);

/// A stored value that cannot be turned into a job field
struct MalformedField {
    field: &'static str,
    detail: &'static str,
}

fn parse_uuid(value: &str, field: &'static str) -> std::result::Result<Uuid, MalformedField> {
    Uuid::parse_str(value).map_err(|_| MalformedField {
        field,
        detail: "not a valid id",
    })
}

fn parse_timestamp(
    value: &str,
    field: &'static str,
) -> std::result::Result<DateTime<Utc>, MalformedField> {
    DateTime::parse_from_rfc3339(value)
        .map(Into::into)
        .map_err(|_| MalformedField {
            field,
            detail: "not a valid timestamp",
        })
}

fn job_from_claimed(row: ClaimedRow) -> std::result::Result<Job, MalformedField> {
    let (
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
        trace_parent,
        trace_state,
    ) = row;
    let job_type: JobType = serde_json::from_str(&payload).map_err(|e| MalformedField {
        field: "payload",
        detail: payload_problem(&e),
    })?;
    Ok(Job {
        id: parse_uuid(&id, "id")?,
        claim_id: claim_id.and_then(|id| Uuid::parse_str(&id).ok()),
        lease_expires_at,
        trace: super::JobTraceContext::from_columns(trace_parent, trace_state),
        job_type,
        status: JobStatus::Processing,
        attempts,
        max_attempts,
        error,
        created_at: parse_timestamp(&created_at, "created_at")?,
        updated_at: parse_timestamp(&updated_at, "updated_at")?,
        scheduled_at: parse_timestamp(&scheduled_at, "scheduled_at")?,
        started_at: started_at.and_then(|s| parse_timestamp(&s, "started_at").ok()),
        completed_at: completed_at.and_then(|s| parse_timestamp(&s, "completed_at").ok()),
    })
}

/// A job row as returned when listing active jobs for a document
type ActiveRow = (
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
    Option<String>,
    Option<String>,
);

fn active_job_from_row(row: ActiveRow) -> std::result::Result<Job, MalformedField> {
    let (
        id,
        _job_type,
        payload,
        status,
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
        trace_parent,
        trace_state,
    ) = row;
    let mut job = job_from_claimed((
        id,
        String::new(),
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
        trace_parent,
        trace_state,
    ))?;
    job.status = JobStatus::from(status);
    Ok(job)
}

/// SQLite-backed job queue
pub(crate) async fn insert_current_processing(
    conn: &mut SqliteConnection,
    vault: folio_core::models::VaultId,
    document_id: Uuid,
    force: bool,
    trace: Option<&super::JobTraceContext>,
) -> Result<Uuid> {
    let document = super::processing_scope::sqlite::document(conn, document_id).await?;
    let job = super::processing_scope::bind_requested_processing(
        document.as_ref(),
        vault,
        document_id,
        force,
        super::REVISION_DISPATCH_ENABLED,
    )?;
    insert_job(conn, job, Utc::now(), DEFAULT_MAX_ATTEMPTS, trace).await
}

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
    #[instrument(skip(self, trace))]
    pub async fn enqueue(
        &self,
        job_type: JobType,
        trace: Option<super::JobTraceContext>,
    ) -> Result<Uuid> {
        self.enqueue_delayed(job_type, Utc::now(), trace).await
    }

    /// Enqueue a job to run at a specific time
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

    pub(super) async fn enqueue_current_processing(
        &self,
        vault: folio_core::models::VaultId,
        document_id: Uuid,
        force: bool,
        trace: Option<super::JobTraceContext>,
    ) -> Result<Uuid> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|error| Error::Repository(error.to_string()))?;
        let id =
            insert_current_processing(&mut tx, vault, document_id, force, trace.as_ref()).await?;
        tx.commit()
            .await
            .map_err(|error| Error::Repository(error.to_string()))?;
        Ok(id)
    }

    /// Enqueue a job with its own retry budget
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
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|error| Error::Repository(error.to_string()))?;
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
            .map_err(|error| Error::Repository(error.to_string()))?;
        Ok(id)
    }

    /// Dequeue the next available job
    ///
    /// A row that cannot be read is marked failed and skipped, so one bad row never blocks the
    /// jobs behind it.
    pub async fn dequeue(&self, class: Option<JobClass>) -> Result<Option<Job>> {
        for _ in 0..MAX_UNREADABLE_PER_DEQUEUE {
            let Some(row) = self.claim_next(class).await? else {
                return Ok(None);
            };
            let id = row.0.clone();
            let claim_id = row.11.clone();
            match job_from_claimed(row) {
                Ok(job) => return Ok(Some(job)),
                Err(bad) => self.quarantine(&id, claim_id.as_deref(), &bad).await?,
            }
        }
        warn!("Set aside {MAX_UNREADABLE_PER_DEQUEUE} unreadable job rows in one call, pausing");
        Ok(None)
    }

    async fn claim_next(&self, class: Option<JobClass>) -> Result<Option<ClaimedRow>> {
        let now = Utc::now();
        let class_filter = class.map_or("TRUE", JobClass::condition);

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
            RETURNING id, job_type, payload, attempts, max_attempts, error, created_at, updated_at, scheduled_at, started_at, completed_at, claim_id, lease_expires_at, trace_parent, trace_state
            "#,
        )))
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .bind(Uuid::new_v4().to_string())
        .bind(LEASE_SECONDS)
        .bind(now.to_rfc3339())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to dequeue job: {}", e)))
    }

    /// Take an unreadable row out of circulation, keeping it for an operator to inspect
    async fn quarantine(
        &self,
        id: &str,
        claim_id: Option<&str>,
        bad: &MalformedField,
    ) -> Result<()> {
        warn!(job_id = %id, field = bad.field, "Skipping job row with an unreadable field");
        let message = damaged_row_message(bad.field, bad.detail);
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE jobs SET status = 'failed', error = ?, updated_at = ?, completed_at = ?, claim_id = NULL, lease_expires_at = NULL WHERE id = ? AND claim_id = ?",
        )
        .bind(message)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .bind(claim_id)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Repository(format!("Failed to quarantine job: {e}")))?;
        Ok(())
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
        let row: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT origin_vault_id, correlation_id, causation_id FROM jobs WHERE id = ?",
        )
        .bind(job_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| Error::Repository(e.to_string()))?;
        let Some((vault, Some(correlation), causation)) = row else {
            return Ok(None);
        };
        let parse = |id: String| Uuid::parse_str(&id).map_err(|e| Error::Repository(e.to_string()));
        Ok(Some(super::JobContext {
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
    pub async fn find_active_jobs_for_document(&self, document_id: Uuid) -> Result<Vec<Job>> {
        // The document_id is stored in the JSON payload, so we search with LIKE
        let search_pattern = format!("%\"document_id\":\"{}\"%", document_id);

        let rows: Vec<ActiveRow> = sqlx::query_as(
            r#"
            SELECT id, job_type, payload, status, attempts, max_attempts, error,
                   created_at, updated_at, scheduled_at, started_at, completed_at, claim_id, lease_expires_at,
                   trace_parent, trace_state
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
            let id = row.0.clone();
            match active_job_from_row(row) {
                Ok(job) => jobs.push(job),
                Err(bad) => {
                    warn!(job_id = %id, field = bad.field, "Ignoring job row with an unreadable field");
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::JobQueue;

    async fn queue() -> (JobQueue, SqlitePool) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::database::run_migrations(&pool).await.unwrap();
        (JobQueue::sqlite(pool.clone()), pool)
    }

    /// Insert a pending row that sorts ahead of any job enqueued now
    async fn insert_row(
        pool: &SqlitePool,
        id: &str,
        payload: &str,
        created_at: &str,
        scheduled_at: &str,
    ) {
        sqlx::query(
            "INSERT INTO jobs (id, job_type, payload, status, attempts, max_attempts, created_at, updated_at, scheduled_at) VALUES (?, 'extract_text', ?, 'pending', 0, 3, ?, ?, ?)",
        )
        .bind(id)
        .bind(payload)
        .bind(created_at)
        .bind(created_at)
        .bind(scheduled_at)
        .execute(pool)
        .await
        .unwrap();
    }

    const EARLY: &str = "2000-01-01T00:00:00+00:00";

    fn good_payload() -> String {
        serde_json::to_string(&JobType::extract_text(Uuid::new_v4())).unwrap()
    }

    async fn assert_skipped_and_failed(id: &str, field: &str, queue: &JobQueue, pool: &SqlitePool) {
        let good = queue
            .enqueue(JobType::extract_text(Uuid::new_v4()))
            .await
            .unwrap();
        let job = queue.dequeue().await.unwrap().unwrap();
        assert_eq!(job.id, good);
        assert!(queue.dequeue().await.unwrap().is_none());

        let (status, error): (String, Option<String>) =
            sqlx::query_as("SELECT status, error FROM jobs WHERE id = ?")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(status, "failed");
        let error = error.unwrap();
        assert!(error.contains(field), "{error}");
        assert!(!error.contains("serde") && error.len() < 120, "{error}");
    }

    #[tokio::test]
    async fn stored_error_never_quotes_the_payload() {
        let (queue, pool) = queue().await;
        let id = Uuid::new_v4().to_string();
        insert_row(
            &pool,
            &id,
            r#"{"type":"from_the_future","data":{"secret":"hunter2"}}"#,
            EARLY,
            EARLY,
        )
        .await;
        assert!(queue.dequeue().await.unwrap().is_none());
        let (error,): (String,) = sqlx::query_as("SELECT error FROM jobs WHERE id = ?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            error,
            "Job row is damaged: its payload could not be read (unknown job type)"
        );
    }

    #[tokio::test]
    async fn bad_updated_at_is_replaced_when_the_job_is_claimed() {
        let (queue, pool) = queue().await;
        let id = Uuid::new_v4().to_string();
        insert_row(&pool, &id, &good_payload(), EARLY, EARLY).await;
        sqlx::query("UPDATE jobs SET updated_at = 'garbage' WHERE id = ?")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        let job = queue.dequeue().await.unwrap().unwrap();
        assert_eq!(job.id.to_string(), id);
    }

    #[tokio::test]
    async fn one_dequeue_sets_aside_a_bounded_number_of_unreadable_rows() {
        let (queue, pool) = queue().await;
        for _ in 0..=MAX_UNREADABLE_PER_DEQUEUE {
            insert_row(
                &pool,
                &Uuid::new_v4().to_string(),
                "{not json",
                EARLY,
                EARLY,
            )
            .await;
        }
        let good = queue
            .enqueue(JobType::extract_text(Uuid::new_v4()))
            .await
            .unwrap();
        assert!(queue.dequeue().await.unwrap().is_none());
        assert_eq!(queue.dequeue().await.unwrap().unwrap().id, good);
    }

    #[tokio::test]
    async fn expired_claim_with_an_unreadable_id_is_failed_by_recovery() {
        let (queue, pool) = queue().await;
        insert_row(&pool, "not-a-uuid", &good_payload(), EARLY, EARLY).await;
        sqlx::query("UPDATE jobs SET status = 'processing', lease_expires_at = 0")
            .execute(&pool)
            .await
            .unwrap();
        queue.reset_stuck().await.unwrap();
        let (status, error): (String, Option<String>) =
            sqlx::query_as("SELECT status, error FROM jobs WHERE id = 'not-a-uuid'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "failed");
        assert!(error.unwrap().contains("id"));
    }

    #[tokio::test]
    async fn quarantine_leaves_a_row_claimed_by_someone_else() {
        let (queue, pool) = queue().await;
        let id = Uuid::new_v4().to_string();
        insert_row(&pool, &id, "{not json", EARLY, EARLY).await;
        sqlx::query("UPDATE jobs SET status = 'processing', claim_id = 'other'")
            .execute(&pool)
            .await
            .unwrap();
        let JobQueue::Sqlite(queue) = &queue else {
            unreachable!()
        };
        let bad = MalformedField {
            field: "payload",
            detail: "not valid JSON",
        };
        queue.quarantine(&id, Some("mine"), &bad).await.unwrap();
        let (status,): (String,) = sqlx::query_as("SELECT status FROM jobs WHERE id = ?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "processing");
    }

    #[tokio::test]
    async fn bad_id_is_failed_and_skipped() {
        let (queue, pool) = queue().await;
        insert_row(&pool, "not-a-uuid", &good_payload(), EARLY, EARLY).await;
        assert_skipped_and_failed("not-a-uuid", "id", &queue, &pool).await;
    }

    #[tokio::test]
    async fn undecodable_payload_is_failed_and_skipped() {
        let (queue, pool) = queue().await;
        let id = Uuid::new_v4().to_string();
        insert_row(&pool, &id, "{not json", EARLY, EARLY).await;
        assert_skipped_and_failed(&id, "payload", &queue, &pool).await;
    }

    #[tokio::test]
    async fn unknown_job_kind_is_failed_and_skipped() {
        let (queue, pool) = queue().await;
        let id = Uuid::new_v4().to_string();
        insert_row(
            &pool,
            &id,
            r#"{"type":"from_the_future","data":{}}"#,
            EARLY,
            EARLY,
        )
        .await;
        assert_skipped_and_failed(&id, "payload", &queue, &pool).await;
    }

    #[tokio::test]
    async fn bad_created_at_is_failed_and_skipped() {
        let (queue, pool) = queue().await;
        let id = Uuid::new_v4().to_string();
        insert_row(&pool, &id, &good_payload(), "yesterday-ish", EARLY).await;
        assert_skipped_and_failed(&id, "created_at", &queue, &pool).await;
    }

    #[tokio::test]
    async fn bad_scheduled_at_is_failed_and_skipped() {
        let (queue, pool) = queue().await;
        let id = Uuid::new_v4().to_string();
        insert_row(&pool, &id, &good_payload(), EARLY, "0-bad").await;
        assert_skipped_and_failed(&id, "scheduled_at", &queue, &pool).await;
    }

    #[tokio::test]
    async fn listing_active_jobs_ignores_unreadable_rows() {
        let (queue, pool) = queue().await;
        let document = Uuid::new_v4();
        queue
            .enqueue(JobType::extract_text(document))
            .await
            .unwrap();
        let payload = serde_json::to_string(&JobType::extract_text(document)).unwrap();
        insert_row(&pool, "not-a-uuid", &payload, EARLY, EARLY).await;
        let jobs = queue.find_active_jobs_for_document(document).await.unwrap();
        assert_eq!(jobs.len(), 1);
    }
}
