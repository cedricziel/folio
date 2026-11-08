use chrono::Utc;
use folio_core::models::{CreateDocument, VaultId};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};
use uuid::Uuid;

use super::{prepare_claim, with_scope, ClaimPreparation, ProcessingScope};
use crate::database::Database;
use crate::queue::{Job, JobQueue, JobType, DEFAULT_MAX_ATTEMPTS};

async fn execute(db: &Database, sql: String) {
    match db {
        Database::Sqlite(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(pool)
            .await
            .map(|_| ()),
        Database::Postgres(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(pool)
            .await
            .map(|_| ()),
    }
    .unwrap();
}

async fn count(db: &Database, sql: String) -> i64 {
    match db {
        Database::Sqlite(pool) => {
            sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
        }
        Database::Postgres(pool) => {
            sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
        }
    }
    .unwrap()
}

struct Fixture<'a> {
    db: &'a Database,
    queue: JobQueue,
    document: Uuid,
    vault: VaultId,
}

impl<'a> Fixture<'a> {
    async fn new(db: &'a Database) -> Self {
        execute(
            db,
            "DELETE FROM processing_stage_runs; DELETE FROM jobs;".into(),
        )
        .await;
        let repos = db.repositories().await;
        let vault = repos
            .vaults
            .create("Processing queue contract", None)
            .await
            .unwrap()
            .id;
        let document = repos
            .documents
            .create(
                vault,
                CreateDocument {
                    title: "Queued document".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .id;
        Self {
            db,
            queue: repos.jobs,
            document,
            vault,
        }
    }

    async fn enqueue_root(&self) -> Uuid {
        self.queue
            .enqueue(JobType::process_document(self.document))
            .await
            .unwrap()
    }

    async fn claim_root(&self) -> (Job, ProcessingScope) {
        let id = self.enqueue_root().await;
        let job = self.queue.dequeue().await.unwrap().unwrap();
        assert_eq!(job.id, id);
        let scope = ready_scope(&self.queue, &job).await;
        (job, scope)
    }

    async fn rows(&self, condition: &str) -> i64 {
        count(self.db, format!("SELECT COUNT(*) FROM processing_stage_runs WHERE document_id = '{}' AND {condition}", self.document)).await
    }

    async fn ocr(&self, scope: ProcessingScope) -> folio_core::Result<Uuid> {
        with_scope(
            Some(scope),
            self.queue.enqueue(JobType::extract_text(self.document)),
        )
        .await
    }

    async fn legacy(&self) -> Uuid {
        let id = self.enqueue_root().await;
        execute(self.db, format!("DELETE FROM processing_stage_runs WHERE document_id = '{}'; UPDATE jobs SET processing_group_id = NULL, origin_vault_id = NULL, correlation_id = NULL, causation_id = NULL WHERE id = '{id}'", self.document)).await;
        id
    }
}

async fn ready_scope(queue: &JobQueue, job: &Job) -> ProcessingScope {
    match prepare_claim(queue, job).await.unwrap() {
        ClaimPreparation::Ready(Some(scope)) => scope,
        _ => panic!("a live current processing claim must have a scope"),
    }
}

async fn root_supersedes_all_stages(db: &Database) {
    let f = Fixture::new(db).await;
    let old = f.enqueue_root().await;
    let new = f.enqueue_root().await;
    assert_eq!(
        f.rows(&format!("group_id = '{old}' AND NOT is_current"))
            .await,
        3
    );
    assert_eq!(f.rows(&format!("group_id = '{new}' AND is_current AND revision_id IS NULL AND state = 'pending' AND attempt = 0 AND queue_attempt = 0")).await, 3);
    assert_eq!(
        f.rows(&format!(
            "group_id = '{new}' AND stage = 'thumbnail' AND job_id = '{new}'"
        ))
        .await,
        1
    );
    assert_eq!(
        f.rows(&format!(
            "group_id = '{new}' AND stage IN ('ocr', 'embedding') AND job_id IS NULL"
        ))
        .await,
        2
    );
    assert_eq!(
        count(
            db,
            format!(
                "SELECT COUNT(*) FROM jobs WHERE id = '{new}' AND processing_group_id = '{new}'"
            )
        )
        .await,
        1
    );
}

async fn standalone_jobs_seed_only_their_planned_stages(db: &Database) {
    let f = Fixture::new(db).await;
    let ocr = f
        .queue
        .enqueue(JobType::extract_text(f.document))
        .await
        .unwrap();
    assert_eq!(
        f.rows(&format!("group_id = '{ocr}' AND is_current")).await,
        2
    );
    assert_eq!(
        f.rows(&format!(
            "group_id = '{ocr}' AND stage = 'ocr' AND job_id = '{ocr}'"
        ))
        .await,
        1
    );
    assert_eq!(
        f.rows(&format!(
            "group_id = '{ocr}' AND stage = 'embedding' AND job_id IS NULL"
        ))
        .await,
        1
    );
    let thumbnail = f
        .queue
        .enqueue(JobType::GenerateThumbnail {
            document_id: f.document,
        })
        .await
        .unwrap();
    let embedding = f
        .queue
        .enqueue(JobType::generate_embedding(f.document))
        .await
        .unwrap();
    assert_eq!(
        f.rows(&format!(
            "group_id = '{thumbnail}' AND job_id = '{thumbnail}' AND stage = 'thumbnail'"
        ))
        .await,
        1
    );
    assert_eq!(
        f.rows(&format!(
            "group_id = '{embedding}' AND job_id = '{embedding}' AND stage = 'embedding'"
        ))
        .await,
        1
    );
    f.queue
        .enqueue(JobType::PurgeDocument {
            document_id: f.document,
            correlation_id: None,
            causation_id: None,
        })
        .await
        .unwrap();
    f.queue
        .enqueue(JobType::extract_text(Uuid::new_v4()))
        .await
        .unwrap();
    assert_eq!(
        count(db, "SELECT COUNT(*) FROM processing_stage_runs".into()).await,
        4
    );
}

async fn child_enqueue_assigns_once_across_queue_claims(db: &Database) {
    let f = Fixture::new(db).await;
    let (root, scope) = f.claim_root().await;
    let (first, duplicate) = tokio::join!(f.ocr(scope), f.ocr(scope));
    let child = first.unwrap();
    assert_eq!(child, duplicate.unwrap());
    assert_eq!(f.rows("is_current").await, 3);
    assert_eq!(
        f.rows(&format!(
            "stage = 'ocr' AND job_id = '{child}' AND group_id = '{}'",
            root.id
        ))
        .await,
        1
    );
    assert_eq!(count(db, "SELECT COUNT(*) FROM jobs".into()).await, 2);
    assert!(f.queue.complete(&root).await.unwrap());
    let job = f.queue.dequeue().await.unwrap().unwrap();
    assert_eq!(job.id, child);
    let child_scope = ready_scope(&f.queue, &job).await;
    assert_eq!(child_scope.group_id, root.id);
    let embedding = with_scope(
        Some(child_scope),
        f.queue.enqueue(JobType::generate_embedding(f.document)),
    )
    .await
    .unwrap();
    let duplicate = with_scope(
        Some(child_scope),
        f.queue.enqueue(JobType::generate_embedding(f.document)),
    )
    .await
    .unwrap();
    assert_eq!(embedding, duplicate);
    assert_eq!(count(db, "SELECT COUNT(*) FROM jobs".into()).await, 3);
    assert_eq!(
        f.queue.context(embedding).await.unwrap().unwrap().vault_id,
        Some(f.vault)
    );
    assert_eq!(f.rows(&format!("group_id = '{}'", root.id)).await, 3);
}

async fn expired_parent_cannot_schedule_children(db: &Database) {
    let f = Fixture::new(db).await;
    let (job, scope) = f.claim_root().await;
    execute(
        db,
        format!(
            "UPDATE jobs SET lease_expires_at = 0 WHERE id = '{}'",
            job.id
        ),
    )
    .await;
    assert!(f.ocr(scope).await.is_err());
    assert_eq!(count(db, "SELECT COUNT(*) FROM jobs".into()).await, 1);
    assert_eq!(f.rows("stage = 'ocr' AND job_id IS NULL").await, 1);
    assert!(matches!(
        prepare_claim(&f.queue, &job).await.unwrap(),
        ClaimPreparation::Obsolete
    ));
}

async fn replaced_parent_claim_cannot_schedule_children(db: &Database) {
    let f = Fixture::new(db).await;
    let (job, scope) = f.claim_root().await;
    execute(
        db,
        format!(
            "UPDATE jobs SET lease_expires_at = 0 WHERE id = '{}'",
            job.id
        ),
    )
    .await;
    f.queue.reset_stuck().await.unwrap();
    let replacement = f.queue.dequeue().await.unwrap().unwrap();
    assert_eq!(replacement.id, job.id);
    assert_ne!(replacement.claim_id, job.claim_id);
    assert!(f.ocr(scope).await.is_err());
    let scope = ready_scope(&f.queue, &replacement).await;
    assert!(f.ocr(scope).await.is_ok());
    assert_eq!(count(db, "SELECT COUNT(*) FROM jobs".into()).await, 2);
}

async fn completed_parent_cannot_schedule_children(db: &Database) {
    let f = Fixture::new(db).await;
    let (job, scope) = f.claim_root().await;
    assert!(f.queue.complete(&job).await.unwrap());
    assert!(f.ocr(scope).await.is_err());
    assert_eq!(count(db, "SELECT COUNT(*) FROM jobs".into()).await, 1);
}

async fn superseded_parent_cannot_displace_new_work(db: &Database) {
    let f = Fixture::new(db).await;
    let (job, scope) = f.claim_root().await;
    let current = f.enqueue_root().await;
    assert!(f.ocr(scope).await.is_err());
    assert!(matches!(
        prepare_claim(&f.queue, &job).await.unwrap(),
        ClaimPreparation::Obsolete
    ));
    assert_eq!(
        f.rows(&format!(
            "group_id = '{current}' AND is_current AND job_id IS NULL"
        ))
        .await,
        2
    );
    assert_eq!(count(db, "SELECT COUNT(*) FROM jobs".into()).await, 2);
}

async fn moved_document_retains_original_audience(db: &Database) {
    let f = Fixture::new(db).await;
    let (job, scope) = f.claim_root().await;
    let destination = db
        .repositories()
        .await
        .vaults
        .create("Destination", None)
        .await
        .unwrap()
        .id;
    execute(
        db,
        format!(
            "UPDATE documents SET vault_id = '{destination}' WHERE id = '{}'",
            f.document
        ),
    )
    .await;
    assert!(f.ocr(scope).await.is_err());
    assert!(matches!(
        prepare_claim(&f.queue, &job).await.unwrap(),
        ClaimPreparation::Obsolete
    ));
    assert_eq!(
        f.queue.context(job.id).await.unwrap().unwrap().vault_id,
        Some(f.vault)
    );
    assert_eq!(f.rows(&format!("vault_id = '{}'", f.vault)).await, 3);
    assert_eq!(count(db, "SELECT COUNT(*) FROM jobs".into()).await, 1);
}

async fn unrelated_processing_roots_do_not_inherit_group(db: &Database) {
    let f = Fixture::new(db).await;
    let (job, scope) = f.claim_root().await;
    let other_document = db
        .repositories()
        .await
        .documents
        .create(
            f.vault,
            CreateDocument {
                title: "Other document".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;
    let other = with_scope(
        Some(scope),
        f.queue.enqueue(JobType::extract_text(other_document)),
    )
    .await
    .unwrap();
    let fresh = with_scope(
        Some(scope),
        f.queue.enqueue(JobType::process_document(f.document)),
    )
    .await
    .unwrap();
    assert_ne!(other, job.id);
    assert_ne!(fresh, job.id);
    assert_eq!(
        count(
            db,
            format!("SELECT COUNT(*) FROM jobs WHERE id = '{other}' AND processing_group_id = '{other}'")
        )
        .await,
        1
    );
    assert_eq!(
        count(
            db,
            format!("SELECT COUNT(*) FROM jobs WHERE id = '{fresh}' AND processing_group_id = '{fresh}'")
        )
        .await,
        1
    );
}

async fn root_enqueue_rolls_back_job_and_supersession(db: &Database) {
    let f = Fixture::new(db).await;
    let previous = f.enqueue_root().await;
    match db {
        Database::Sqlite(pool) => {
            let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
            crate::queue::insert_job_sqlite(
                &mut tx,
                JobType::process_document(f.document),
                Utc::now(),
                DEFAULT_MAX_ATTEMPTS,
                None,
            )
            .await
            .unwrap();
            tx.rollback().await.unwrap();
        }
        Database::Postgres(pool) => {
            let mut tx = pool.begin().await.unwrap();
            crate::queue::insert_job_postgres(
                &mut tx,
                JobType::process_document(f.document),
                Utc::now(),
                DEFAULT_MAX_ATTEMPTS,
                None,
            )
            .await
            .unwrap();
            tx.rollback().await.unwrap();
        }
    }
    assert_eq!(count(db, "SELECT COUNT(*) FROM jobs".into()).await, 1);
    assert_eq!(
        f.rows(&format!("group_id = '{previous}' AND is_current"))
            .await,
        3
    );
    assert_eq!(f.rows("TRUE").await, 3);
}

async fn child_enqueue_rolls_back_assignment_with_job(db: &Database) {
    let f = Fixture::new(db).await;
    let (_, scope) = f.claim_root().await;
    with_scope(Some(scope), async {
        match db {
            Database::Sqlite(pool) => {
                let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
                crate::queue::insert_job_sqlite(
                    &mut tx,
                    JobType::extract_text(f.document),
                    Utc::now(),
                    DEFAULT_MAX_ATTEMPTS,
                    None,
                )
                .await
                .unwrap();
                tx.rollback().await.unwrap();
            }
            Database::Postgres(pool) => {
                let mut tx = pool.begin().await.unwrap();
                crate::queue::insert_job_postgres(
                    &mut tx,
                    JobType::extract_text(f.document),
                    Utc::now(),
                    DEFAULT_MAX_ATTEMPTS,
                    None,
                )
                .await
                .unwrap();
                tx.rollback().await.unwrap();
            }
        }
    })
    .await;
    assert_eq!(count(db, "SELECT COUNT(*) FROM jobs".into()).await, 1);
    assert_eq!(f.rows("stage = 'ocr' AND job_id IS NULL").await, 1);
    assert!(f.ocr(scope).await.is_ok());
}

async fn legacy_claim_adopts_group_and_new_lineage_once(db: &Database) {
    let f = Fixture::new(db).await;
    let id = f.legacy().await;
    let job = f.queue.dequeue().await.unwrap().unwrap();
    assert_eq!(job.id, id);
    let first = ready_scope(&f.queue, &job).await;
    let context = f.queue.context(id).await.unwrap().unwrap();
    let again = ready_scope(&f.queue, &job).await;
    assert_eq!(first.group_id, id);
    assert_eq!(again.group_id, first.group_id);
    assert_eq!(context, f.queue.context(id).await.unwrap().unwrap());
    assert_eq!(context.vault_id, Some(f.vault));
    assert_eq!(context.lineage.causation_id, None);
    assert_eq!(
        f.rows(&format!(
            "group_id = '{id}' AND revision_id IS NULL AND is_current"
        ))
        .await,
        3
    );
}

async fn legacy_claim_does_not_displace_a_newer_group(db: &Database) {
    let f = Fixture::new(db).await;
    let old = f.legacy().await;
    let newer = f.enqueue_root().await;
    let job = f.queue.dequeue().await.unwrap().unwrap();
    assert_eq!(job.id, old);
    assert!(matches!(
        prepare_claim(&f.queue, &job).await.unwrap(),
        ClaimPreparation::Obsolete
    ));
    assert!(f.queue.context(old).await.unwrap().is_none());
    assert_eq!(
        f.rows(&format!("group_id = '{newer}' AND is_current"))
            .await,
        3
    );
    assert_eq!(f.rows("TRUE").await, 3);
}

async fn partial_supersession_keeps_the_current_parent_stage_live(db: &Database) {
    let f = Fixture::new(db).await;
    let id = f
        .queue
        .enqueue(JobType::extract_text(f.document))
        .await
        .unwrap();
    let job = f.queue.dequeue().await.unwrap().unwrap();
    assert_eq!(job.id, id);
    let replacement_embedding = f
        .queue
        .enqueue(JobType::generate_embedding(f.document))
        .await
        .unwrap();
    let scope = ready_scope(&f.queue, &job).await;
    assert_eq!(scope.group_id, id);
    assert_eq!(
        f.rows(&format!(
            "group_id = '{id}' AND stage = 'ocr' AND is_current"
        ))
        .await,
        1
    );
    assert!(with_scope(
        Some(scope),
        f.queue.enqueue(JobType::generate_embedding(f.document))
    )
    .await
    .is_err());
    assert_eq!(
        f.rows(&format!(
            "group_id = '{replacement_embedding}' AND stage = 'embedding' AND is_current"
        ))
        .await,
        1
    );
}

async fn legacy_claim_rejects_a_forged_in_memory_document(db: &Database) {
    let f = Fixture::new(db).await;
    f.legacy().await;
    let job = f.queue.dequeue().await.unwrap().unwrap();
    let other_document = db
        .repositories()
        .await
        .documents
        .create(
            f.vault,
            CreateDocument {
                title: "Forged target".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;
    let mut forged = job.clone();
    forged.job_type = JobType::process_document(other_document);
    assert!(matches!(
        prepare_claim(&f.queue, &forged).await.unwrap(),
        ClaimPreparation::Obsolete
    ));
    assert_eq!(
        count(db, "SELECT COUNT(*) FROM processing_stage_runs".into()).await,
        0
    );
    assert!(f.queue.context(job.id).await.unwrap().is_none());
    assert_eq!(ready_scope(&f.queue, &job).await.document_id, f.document);
}

async fn legacy_claim_rejects_a_forged_in_memory_job_type(db: &Database) {
    let f = Fixture::new(db).await;
    f.legacy().await;
    let job = f.queue.dequeue().await.unwrap().unwrap();
    let mut forged = job.clone();
    forged.job_type = JobType::generate_embedding(f.document);
    assert!(matches!(
        prepare_claim(&f.queue, &forged).await.unwrap(),
        ClaimPreparation::Obsolete
    ));
    assert_eq!(f.rows("TRUE").await, 0);
    assert!(f.queue.context(job.id).await.unwrap().is_none());
}

async fn legacy_claim_rejects_disagreement_between_columns_and_payload(db: &Database) {
    let f = Fixture::new(db).await;
    let id = f.legacy().await;
    let job = f.queue.dequeue().await.unwrap().unwrap();
    execute(
        db,
        format!("UPDATE jobs SET job_type = 'generate_embedding' WHERE id = '{id}'"),
    )
    .await;
    let mut mismatched = job.clone();
    mismatched.job_type = JobType::generate_embedding(f.document);
    assert!(matches!(
        prepare_claim(&f.queue, &mismatched).await.unwrap(),
        ClaimPreparation::Obsolete
    ));
    assert_eq!(f.rows("TRUE").await, 0);
}

async fn purge_requested_parent_cannot_seed_or_schedule_work(db: &Database) {
    let f = Fixture::new(db).await;
    let (job, scope) = f.claim_root().await;
    execute(
        db,
        format!(
            "UPDATE documents SET purge_requested_at = '2026-11-03T00:00:00Z' WHERE id = '{}'",
            f.document
        ),
    )
    .await;
    assert!(f.ocr(scope).await.is_err());
    assert!(matches!(
        prepare_claim(&f.queue, &job).await.unwrap(),
        ClaimPreparation::Obsolete
    ));
    assert_eq!(count(db, "SELECT COUNT(*) FROM jobs".into()).await, 1);
    assert_eq!(f.rows("stage = 'ocr' AND job_id IS NULL").await, 1);
}

async fn legacy_purge_requested_claim_cannot_adopt_a_group(db: &Database) {
    let f = Fixture::new(db).await;
    f.legacy().await;
    let job = f.queue.dequeue().await.unwrap().unwrap();
    execute(
        db,
        format!(
            "UPDATE documents SET purge_requested_at = '2026-11-03T00:00:00Z' WHERE id = '{}'",
            f.document
        ),
    )
    .await;
    assert!(matches!(
        prepare_claim(&f.queue, &job).await.unwrap(),
        ClaimPreparation::Obsolete
    ));
    assert_eq!(f.rows("TRUE").await, 0);
    assert!(f.queue.context(job.id).await.unwrap().is_none());
}

async fn contract(db: &Database) {
    db.migrate().await.unwrap();
    partial_supersession_keeps_the_current_parent_stage_live(db).await;
    legacy_claim_rejects_a_forged_in_memory_document(db).await;
    legacy_claim_rejects_a_forged_in_memory_job_type(db).await;
    legacy_claim_rejects_disagreement_between_columns_and_payload(db).await;
    purge_requested_parent_cannot_seed_or_schedule_work(db).await;
    legacy_purge_requested_claim_cannot_adopt_a_group(db).await;
    root_supersedes_all_stages(db).await;
    standalone_jobs_seed_only_their_planned_stages(db).await;
    child_enqueue_assigns_once_across_queue_claims(db).await;
    expired_parent_cannot_schedule_children(db).await;
    replaced_parent_claim_cannot_schedule_children(db).await;
    completed_parent_cannot_schedule_children(db).await;
    superseded_parent_cannot_displace_new_work(db).await;
    moved_document_retains_original_audience(db).await;
    unrelated_processing_roots_do_not_inherit_group(db).await;
    root_enqueue_rolls_back_job_and_supersession(db).await;
    child_enqueue_rolls_back_assignment_with_job(db).await;
    legacy_claim_adopts_group_and_new_lineage_once(db).await;
    legacy_claim_does_not_displace_a_newer_group(db).await;
}

#[tokio::test]
async fn sqlite_processing_scope_contract() {
    contract(&Database::connect("sqlite::memory:").await.unwrap()).await;
}

#[tokio::test]
async fn postgres_processing_scope_contract() {
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
