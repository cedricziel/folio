use chrono::Utc;
use folio_core::events::decode_stored_outbox_payload;
use folio_core::models::{CreateDocument, UserId, VaultId};
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::processing_events::ProcessingReason;
use folio_core::DomainEventEnvelope;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};
use uuid::Uuid;

use crate::database::Database;
use crate::queue::{stages, FailOutcome, Job, JobQueue, JobType};

async fn execute(db: &Database, sqlite: &str, postgres: &str) {
    match db {
        Database::Sqlite(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(sqlite.to_owned()))
            .execute(pool)
            .await
            .map(|_| ()),
        Database::Postgres(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(postgres.to_owned()))
            .execute(pool)
            .await
            .map(|_| ()),
    }
    .unwrap();
}

#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct State {
    stage: String,
    attempt: i64,
    reason: Option<String>,
    retry: bool,
    ocr: String,
    job: String,
    queue_attempt: i32,
    claim: Option<String>,
}

struct Fixture<'a> {
    db: &'a Database,
    queue: JobQueue,
    job: Job,
    document: Uuid,
    vault: VaultId,
    lineage: MutationLineage,
}

impl<'a> Fixture<'a> {
    async fn new(db: &'a Database, max_attempts: i32) -> Self {
        execute(
            db,
            "DELETE FROM processing_stage_runs; DELETE FROM jobs; DELETE FROM outbox_events;",
            "DELETE FROM processing_stage_runs; DELETE FROM jobs; DELETE FROM outbox_events;",
        )
        .await;
        let repos = db.repositories().await;
        let vault = repos
            .vaults
            .create("Recovery source", None)
            .await
            .unwrap()
            .id;
        let document = repos
            .documents
            .create(
                vault,
                CreateDocument {
                    title: "Recovery contract".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .id;
        let lineage = MutationLineage {
            correlation_id: Uuid::new_v4(),
            causation_id: Some(Uuid::new_v4()),
        };
        let id = with_lineage(
            Some(lineage),
            repos.jobs.enqueue_with_max_attempts(
                JobType::extract_text(document),
                Utc::now(),
                max_attempts,
            ),
        )
        .await
        .unwrap();
        let job = repos.jobs.dequeue().await.unwrap().unwrap();
        assert_eq!(job.id, id);
        let fixture = Self {
            db,
            queue: repos.jobs,
            job,
            document,
            vault,
            lineage,
        };
        fixture.start(&fixture.job).await;
        fixture.clear_events().await;
        fixture
    }

    async fn start(&self, job: &Job) {
        let fence = match self.db {
            Database::Sqlite(pool) => stages::sqlite::start_ocr(pool, job).await.unwrap(),
            Database::Postgres(pool) => stages::postgres::start_ocr(pool, job).await.unwrap(),
        };
        assert!(fence.is_some());
    }

    async fn fail(
        &self,
        job: &Job,
        retry: bool,
        reason: ProcessingReason,
    ) -> folio_core::Result<FailOutcome> {
        match self.db {
            Database::Sqlite(pool) => {
                super::sqlite::fail(
                    pool,
                    job,
                    "private provider response with secret payload",
                    retry,
                    reason,
                )
                .await
            }
            Database::Postgres(pool) => {
                super::postgres::fail(
                    pool,
                    job,
                    "private provider response with secret payload",
                    retry,
                    reason,
                )
                .await
            }
        }
    }

    async fn update(&self, sql: String) {
        execute(self.db, &sql, &sql).await;
    }

    async fn clear_events(&self) {
        self.update("DELETE FROM outbox_events".into()).await;
    }

    async fn expire(&self) {
        self.update(format!(
            "UPDATE jobs SET lease_expires_at = 0 WHERE id = '{}'",
            self.job.id
        ))
        .await;
    }

    async fn state(&self) -> State {
        let sql = format!("SELECT r.state AS stage, r.attempt, r.reason_code AS reason, r.retry_at IS NOT NULL AS retry, d.ocr_status AS ocr, j.status AS job, j.attempts AS queue_attempt, CAST(j.claim_id AS TEXT) AS claim FROM processing_stage_runs r JOIN jobs j ON j.id = r.job_id JOIN documents d ON d.id = r.document_id WHERE r.stage = 'ocr' AND j.id = '{}'", self.job.id);
        match self.db {
            Database::Sqlite(pool) => {
                sqlx::query_as(sqlx::AssertSqlSafe(sql))
                    .fetch_one(pool)
                    .await
            }
            Database::Postgres(pool) => {
                sqlx::query_as(sqlx::AssertSqlSafe(sql))
                    .fetch_one(pool)
                    .await
            }
        }
        .unwrap()
    }

    async fn events(&self) -> Vec<DomainEventEnvelope> {
        let payloads: Vec<String> = match self.db {
            Database::Sqlite(pool) => {
                sqlx::query_scalar("SELECT payload FROM outbox_events ORDER BY id")
                    .fetch_all(pool)
                    .await
                    .unwrap()
            }
            Database::Postgres(pool) => {
                sqlx::query_scalar("SELECT payload::text FROM outbox_events ORDER BY id")
                    .fetch_all(pool)
                    .await
                    .unwrap()
            }
        };
        payloads
            .into_iter()
            .map(|payload| {
                decode_stored_outbox_payload(&payload)
                    .unwrap()
                    .envelope
                    .unwrap()
            })
            .filter(|envelope| !envelope.event_type().starts_with("vault"))
            .collect()
    }

    async fn assert_retry_time_matches_queue(&self) {
        let sql = format!("SELECT j.scheduled_at = r.retry_at FROM jobs j JOIN processing_stage_runs r ON r.job_id = j.id WHERE j.id = '{}' AND r.stage = 'ocr'", self.job.id);
        let equal: bool = match self.db {
            Database::Sqlite(pool) => sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
                .unwrap(),
            Database::Postgres(pool) => sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
                .unwrap(),
        };
        assert!(equal);
    }
}

async fn transient_failure_is_atomic_and_preserves_source_lineage(db: &Database) {
    let f = Fixture::new(db, 3).await;
    let unrelated = MutationLineage {
        correlation_id: Uuid::new_v4(),
        causation_id: None,
    };
    assert_eq!(
        with_actor(
            Some(UserId("unrelated-worker-user".into())),
            with_lineage(
                Some(unrelated),
                f.fail(&f.job, true, ProcessingReason::StorageFailed)
            )
        )
        .await
        .unwrap(),
        FailOutcome::Retrying
    );
    let state = f.state().await;
    assert_eq!(
        (
            state.stage.as_str(),
            state.reason.as_deref(),
            state.retry,
            state.ocr.as_str(),
            state.job.as_str(),
            state.claim
        ),
        (
            "waiting_to_retry",
            Some("storage_failed"),
            true,
            "failed",
            "pending",
            None
        )
    );
    assert_eq!(state.attempt, 1);
    f.assert_retry_time_matches_queue().await;
    let events = f.events().await;
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(
        event.event_type(),
        "document.processing.ocr.retry_scheduled"
    );
    assert_eq!(event.data()["reason_code"], "storage_failed");
    assert!(event.data()["retry_at"].is_string());
    assert_eq!(event.correlation_id(), f.lineage.correlation_id);
    assert_eq!(event.causation_id(), f.lineage.causation_id);
    assert_eq!(event.actor_id(), None);
    assert_eq!(event.vault_id(), f.vault);
    assert!(!event.data().to_string().contains("private provider"));
    assert_eq!(
        f.fail(&f.job, true, ProcessingReason::StorageFailed)
            .await
            .unwrap(),
        FailOutcome::LostLease
    );
    assert_eq!(f.events().await.len(), 1);
}

async fn persisted_retry_budget_overrides_forged_job_budget(db: &Database) {
    let f = Fixture::new(db, 1).await;
    let mut forged = f.job.clone();
    forged.max_attempts = i32::MAX;
    assert_eq!(
        f.fail(&forged, true, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::GaveUp
    );
    let state = f.state().await;
    assert_eq!(
        (
            state.stage.as_str(),
            state.job.as_str(),
            state.ocr.as_str(),
            state.retry
        ),
        ("failed", "failed", "failed", false)
    );
    let events = f.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type(), "document.processing.ocr.failed");
    assert_eq!(events[0].data()["reason_code"], "provider_failed");
    assert!(events[0].data().get("retry_at").is_none());
    assert_eq!(
        f.fail(&forged, true, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::LostLease
    );
    assert_eq!(f.events().await.len(), 1);
}

async fn caller_cannot_shorten_the_persisted_retry_budget(db: &Database) {
    let f = Fixture::new(db, 3).await;
    let mut forged = f.job.clone();
    forged.max_attempts = 0;
    assert_eq!(
        f.fail(&forged, true, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::Retrying
    );
    assert_eq!(f.state().await.stage, "waiting_to_retry");
}

async fn explicit_nonretryable_failure_is_terminal(db: &Database) {
    let f = Fixture::new(db, 3).await;
    assert_eq!(
        f.fail(&f.job, false, ProcessingReason::InvalidResponse)
            .await
            .unwrap(),
        FailOutcome::GaveUp
    );
    let state = f.state().await;
    assert_eq!(
        (
            state.stage.as_str(),
            state.reason.as_deref(),
            state.job.as_str(),
            state.retry
        ),
        ("failed", Some("invalid_response"), "failed", false)
    );
    assert_eq!(
        f.events().await[0].event_type(),
        "document.processing.ocr.failed"
    );
}

async fn stale_claim_and_attempt_cannot_record_failure(db: &Database) {
    let f = Fixture::new(db, 3).await;
    let before = f.state().await;
    let mut stale = f.job.clone();
    stale.claim_id = Some(Uuid::new_v4());
    assert_eq!(
        f.fail(&stale, true, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::LostLease
    );
    stale = f.job.clone();
    stale.attempts += 1;
    assert_eq!(
        f.fail(&stale, true, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::LostLease
    );
    assert_eq!(f.state().await, before);
    assert!(f.events().await.is_empty());
    f.expire().await;
    assert_eq!(
        f.fail(&f.job, true, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::LostLease
    );
    assert_eq!(f.state().await.stage, "running");
    assert!(f.events().await.is_empty());
}

async fn expired_lease_recovers_once_with_immediate_retry(db: &Database) {
    let f = Fixture::new(db, 3).await;
    f.expire().await;
    assert_eq!(f.queue.reset_stuck().await.unwrap(), 1);
    assert_eq!(f.queue.reset_stuck().await.unwrap(), 0);
    let state = f.state().await;
    assert_eq!(
        (
            state.stage.as_str(),
            state.reason.as_deref(),
            state.job.as_str(),
            state.ocr.as_str(),
            state.retry
        ),
        (
            "waiting_to_retry",
            Some("lease_expired"),
            "pending",
            "failed",
            true
        )
    );
    f.assert_retry_time_matches_queue().await;
    let events = f.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].event_type(),
        "document.processing.ocr.retry_scheduled"
    );
    assert_eq!(events[0].data()["reason_code"], "lease_expired");
    assert_eq!(events[0].correlation_id(), f.lineage.correlation_id);
    let replacement = f.queue.dequeue().await.unwrap().unwrap();
    assert_eq!(replacement.id, f.job.id);
    assert_ne!(replacement.claim_id, f.job.claim_id);
    assert_eq!(replacement.attempts, 2);
    f.start(&replacement).await;
    assert_eq!(f.state().await.attempt, 2);
    f.clear_events().await;
    let before = f.state().await;
    assert_eq!(
        f.fail(&f.job, false, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::LostLease
    );
    assert_eq!(f.state().await, before);
    assert!(f.events().await.is_empty());
    assert_eq!(
        f.fail(&replacement, false, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::GaveUp
    );
    assert_eq!(f.events().await.len(), 1);
}

async fn last_attempt_crash_is_terminal(db: &Database) {
    let f = Fixture::new(db, 1).await;
    f.expire().await;
    assert_eq!(f.queue.reset_stuck().await.unwrap(), 1);
    assert_eq!(f.queue.reset_stuck().await.unwrap(), 0);
    let state = f.state().await;
    assert_eq!(
        (
            state.stage.as_str(),
            state.reason.as_deref(),
            state.job.as_str(),
            state.ocr.as_str(),
            state.retry
        ),
        ("failed", Some("lease_expired"), "failed", "failed", false)
    );
    assert!(f.queue.dequeue().await.unwrap().is_none());
    let events = f.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type(), "document.processing.ocr.failed");
    assert_eq!(events[0].data()["reason_code"], "lease_expired");
}

async fn live_renewed_claim_is_never_recovered_or_announced(db: &Database) {
    let f = Fixture::new(db, 3).await;
    assert!(f.queue.renew(&f.job).await.unwrap());
    assert_eq!(f.queue.reset_stuck().await.unwrap(), 0);
    assert_eq!(f.state().await.stage, "running");
    assert!(f.events().await.is_empty());
}

async fn refuse_outbox(db: &Database) {
    execute(db,
        "CREATE TRIGGER refuse_recovery_outbox BEFORE INSERT ON outbox_events BEGIN SELECT RAISE(ABORT, 'refused'); END",
        "CREATE FUNCTION refuse_recovery_outbox() RETURNS trigger AS $$ BEGIN RAISE EXCEPTION 'refused'; END $$ LANGUAGE plpgsql; CREATE TRIGGER refuse_recovery_outbox BEFORE INSERT ON outbox_events FOR EACH ROW EXECUTE FUNCTION refuse_recovery_outbox()"
    ).await;
}

async fn allow_outbox(db: &Database) {
    execute(db, "DROP TRIGGER refuse_recovery_outbox", "DROP TRIGGER refuse_recovery_outbox ON outbox_events; DROP FUNCTION refuse_recovery_outbox()").await;
}

async fn outbox_failure_rolls_back_reported_failure(db: &Database) {
    let f = Fixture::new(db, 3).await;
    let before = f.state().await;
    refuse_outbox(db).await;
    assert!(f
        .fail(&f.job, true, ProcessingReason::ProviderFailed)
        .await
        .is_err());
    assert_eq!(f.state().await, before);
    assert!(f.events().await.is_empty());
    allow_outbox(db).await;
    assert_eq!(
        f.fail(&f.job, true, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::Retrying
    );
    assert_eq!(f.events().await.len(), 1);
}

async fn outbox_failure_rolls_back_expired_lease_recovery(db: &Database) {
    let f = Fixture::new(db, 3).await;
    f.expire().await;
    let before = f.state().await;
    refuse_outbox(db).await;
    assert_eq!(f.queue.reset_stuck().await.unwrap(), 0);
    assert_eq!(f.state().await, before);
    assert!(f.events().await.is_empty());
    allow_outbox(db).await;
    assert_eq!(f.queue.reset_stuck().await.unwrap(), 1);
    assert_eq!(f.events().await.len(), 1);
}

async fn moved_stage_is_untouched_by_failure_and_recovery(db: &Database) {
    let f = Fixture::new(db, 3).await;
    let destination = db
        .repositories()
        .await
        .vaults
        .create("Moved recovery destination", None)
        .await
        .unwrap()
        .id;
    f.update(format!(
        "UPDATE documents SET vault_id = '{destination}' WHERE id = '{}'",
        f.document
    ))
    .await;
    f.expire().await;
    assert_eq!(f.queue.reset_stuck().await.unwrap(), 1);
    let state = f.state().await;
    assert_eq!(
        (state.stage.as_str(), state.ocr.as_str(), state.job.as_str()),
        ("running", "running", "pending")
    );
    assert!(f.events().await.is_empty());
    assert_eq!(
        f.queue.context(f.job.id).await.unwrap().unwrap().vault_id,
        Some(f.vault)
    );
}

async fn trashed_stage_is_untouched_by_failure(db: &Database) {
    let f = Fixture::new(db, 3).await;
    f.update(format!(
        "UPDATE documents SET deleted_at = '2026-11-03T00:00:00Z' WHERE id = '{}'",
        f.document
    ))
    .await;
    assert_eq!(
        f.fail(&f.job, true, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::Retrying
    );
    let state = f.state().await;
    assert_eq!(
        (state.stage.as_str(), state.ocr.as_str(), state.job.as_str()),
        ("running", "running", "pending")
    );
    assert!(f.events().await.is_empty());
}

async fn superseded_stage_is_untouched_by_failure(db: &Database) {
    let f = Fixture::new(db, 3).await;
    let newer = f
        .queue
        .enqueue(JobType::extract_text(f.document))
        .await
        .unwrap();
    assert_eq!(
        f.fail(&f.job, true, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::Retrying
    );
    assert_eq!(f.state().await.stage, "running");
    assert_eq!(f.state().await.ocr, "running");
    assert!(f.events().await.is_empty());
    let replacement = f.queue.dequeue().await.unwrap().unwrap();
    assert_eq!(replacement.id, newer);
    f.start(&replacement).await;
    assert_eq!(f.events().await.len(), 1);
}

async fn terminal_stage_is_untouched_by_late_failure(db: &Database) {
    let f = Fixture::new(db, 3).await;
    let completed = match db {
        Database::Sqlite(pool) => {
            let fence = stages::sqlite::start_ocr(pool, &f.job)
                .await
                .unwrap()
                .unwrap();
            stages::sqlite::complete_ocr(pool, &fence, "", true)
                .await
                .unwrap()
        }
        Database::Postgres(pool) => {
            let fence = stages::postgres::start_ocr(pool, &f.job)
                .await
                .unwrap()
                .unwrap();
            stages::postgres::complete_ocr(pool, &fence, "", true)
                .await
                .unwrap()
        }
    };
    assert!(completed);
    f.clear_events().await;
    assert_eq!(
        f.fail(&f.job, true, ProcessingReason::ProviderFailed)
            .await
            .unwrap(),
        FailOutcome::Retrying
    );
    let state = f.state().await;
    assert_eq!(
        (state.stage.as_str(), state.ocr.as_str(), state.reason),
        ("completed", "done", None)
    );
    assert!(f.events().await.is_empty());
}

async fn nonstage_and_prelease_jobs_still_recover(db: &Database) {
    let f = Fixture::new(db, 3).await;
    assert!(f.queue.complete(&f.job).await.unwrap());
    let id = f
        .queue
        .enqueue(JobType::extract_text(Uuid::new_v4()))
        .await
        .unwrap();
    let legacy = f.queue.dequeue().await.unwrap().unwrap();
    assert_eq!(legacy.id, id);
    f.update(format!(
        "UPDATE jobs SET lease_expires_at = NULL, claim_id = NULL WHERE id = '{id}'"
    ))
    .await;
    assert_eq!(f.queue.reset_stuck().await.unwrap(), 1);
    let replacement = f.queue.dequeue().await.unwrap().unwrap();
    assert_eq!(replacement.id, id);
    assert!(f.queue.complete(&replacement).await.unwrap());
    let workflow = f
        .queue
        .enqueue_with_max_attempts(
            JobType::ExecuteWorkflowRun {
                run_id: Uuid::new_v4(),
            },
            Utc::now(),
            1,
        )
        .await
        .unwrap();
    let job = f.queue.dequeue().await.unwrap().unwrap();
    assert_eq!(job.id, workflow);
    f.update(format!(
        "UPDATE jobs SET lease_expires_at = 0 WHERE id = '{workflow}'"
    ))
    .await;
    assert_eq!(f.queue.reset_stuck().await.unwrap(), 1);
    assert!(f.queue.dequeue().await.unwrap().is_none());
    assert_eq!(f.queue.reset_stuck().await.unwrap(), 0);
    assert!(f.events().await.is_empty());
}

async fn contract(db: &Database) {
    db.migrate().await.unwrap();
    transient_failure_is_atomic_and_preserves_source_lineage(db).await;
    persisted_retry_budget_overrides_forged_job_budget(db).await;
    caller_cannot_shorten_the_persisted_retry_budget(db).await;
    explicit_nonretryable_failure_is_terminal(db).await;
    stale_claim_and_attempt_cannot_record_failure(db).await;
    expired_lease_recovers_once_with_immediate_retry(db).await;
    last_attempt_crash_is_terminal(db).await;
    live_renewed_claim_is_never_recovered_or_announced(db).await;
    outbox_failure_rolls_back_reported_failure(db).await;
    outbox_failure_rolls_back_expired_lease_recovery(db).await;
    moved_stage_is_untouched_by_failure_and_recovery(db).await;
    trashed_stage_is_untouched_by_failure(db).await;
    superseded_stage_is_untouched_by_failure(db).await;
    terminal_stage_is_untouched_by_late_failure(db).await;
    nonstage_and_prelease_jobs_still_recover(db).await;
}

#[tokio::test]
async fn sqlite_recovery_contract() {
    contract(&Database::connect("sqlite::memory:").await.unwrap()).await;
}

#[tokio::test]
async fn postgres_recovery_contract() {
    let container = match Postgres::default().with_tag("17-alpine").start().await {
        Ok(container) => container,
        Err(error)
            if std::env::var("FOLIO_REQUIRE_DOCKER")
                .is_ok_and(|value| !value.is_empty() && value != "0") =>
        {
            panic!("FOLIO_REQUIRE_DOCKER is set but PostgreSQL could not start: {error}")
        }
        Err(error) => {
            eprintln!("SKIPPED: PostgreSQL needs a Docker daemon ({error})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    contract(
        &Database::connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .unwrap(),
    )
    .await;
}
