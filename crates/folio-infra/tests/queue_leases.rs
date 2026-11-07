mod common;

use folio_infra::{Database, FailOutcome, Job, JobQueue, JobType};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;
use uuid::Uuid;

async fn claim(queue: &JobQueue) -> Job {
    queue
        .enqueue(JobType::extract_text(Uuid::new_v4()))
        .await
        .unwrap();
    queue.dequeue().await.unwrap().unwrap()
}

async fn set_expiry(db: &Database, job: &Job, expiry: Option<i64>) {
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("UPDATE jobs SET lease_expires_at = ? WHERE id = ?")
                .bind(expiry)
                .bind(job.id.to_string())
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("UPDATE jobs SET lease_expires_at = $1 WHERE id = $2")
                .bind(expiry)
                .bind(job.id)
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

async fn live_long_job_is_not_recovered(db: &Database, queue: &JobQueue) {
    let job = claim(queue).await;
    match db {
        Database::Sqlite(pool) => {
            sqlx::query("UPDATE jobs SET started_at = '2000-01-01T00:00:00Z' WHERE id = ?")
                .bind(job.id.to_string())
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query("UPDATE jobs SET started_at = '2000-01-01T00:00:00Z' WHERE id = $1")
                .bind(job.id)
                .execute(pool)
                .await
                .unwrap();
        }
    }
    let shortened_expiry = job.lease_expires_at.unwrap() - 200;
    set_expiry(db, &job, Some(shortened_expiry)).await;
    assert!(queue.renew(&job).await.unwrap());
    assert_eq!(queue.reset_stuck().await.unwrap(), 0);
    assert!(queue.dequeue().await.unwrap().is_none());
    let active = queue
        .find_active_jobs_for_document(job.job_type.document_id().unwrap())
        .await
        .unwrap();
    assert!(active[0].lease_expires_at.unwrap() > shortened_expiry);
    assert!(queue.complete(&job).await.unwrap());
}

async fn crashed_owner_cannot_change_replacement(db: &Database, queue: &JobQueue) {
    let stale = claim(queue).await;
    set_expiry(db, &stale, Some(0)).await;
    assert!(!queue.renew(&stale).await.unwrap());
    assert!(!queue.complete(&stale).await.unwrap());
    assert_eq!(
        queue.fail(&stale, "expired", false).await.unwrap(),
        FailOutcome::LostLease
    );
    assert_eq!(queue.reset_stuck().await.unwrap(), 1);
    let replacement = queue.dequeue().await.unwrap().unwrap();
    assert_eq!(replacement.id, stale.id);
    assert_ne!(replacement.claim_id, stale.claim_id);
    assert_eq!(replacement.attempts, stale.attempts + 1);
    assert!(!queue.renew(&stale).await.unwrap());
    assert!(!queue.complete(&stale).await.unwrap());
    assert_eq!(
        queue.fail(&stale, "late failure", true).await.unwrap(),
        FailOutcome::LostLease
    );
    assert_eq!(
        queue
            .fail(&stale, "late fatal failure", false)
            .await
            .unwrap(),
        FailOutcome::LostLease
    );
    assert!(queue.renew(&replacement).await.unwrap());
    assert!(queue.complete(&replacement).await.unwrap());
    assert!(!queue.complete(&replacement).await.unwrap());
}

async fn pre_lease_processing_jobs_are_recovered(db: &Database, queue: &JobQueue) {
    let old = claim(queue).await;
    set_expiry(db, &old, None).await;
    assert_eq!(queue.reset_stuck().await.unwrap(), 1);
    let replacement = queue.dequeue().await.unwrap().unwrap();
    assert_eq!(replacement.id, old.id);
    assert!(queue.complete(&replacement).await.unwrap());
}

async fn lease_contract(db: &Database) {
    db.migrate().await.unwrap();
    let queue = db.repositories().await.jobs;
    live_long_job_is_not_recovered(db, &queue).await;
    crashed_owner_cannot_change_replacement(db, &queue).await;
    pre_lease_processing_jobs_are_recovered(db, &queue).await;
}

#[tokio::test]
async fn sqlite_lease_contract() {
    lease_contract(&Database::connect("sqlite::memory:").await.unwrap()).await;
}

#[tokio::test]
async fn postgres_lease_contract() {
    let started = Postgres::default().with_tag("17-alpine").start().await;
    let Some(container) = common::container_or_skip("PostgreSQL", started) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    lease_contract(&db).await;
}
