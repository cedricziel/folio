mod common;

use folio_infra::Database;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;

async fn execute(db: &Database, sql: &str) -> Result<(), sqlx::Error> {
    match db {
        Database::Sqlite(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(pool)
            .await
            .map(|_| ()),
        Database::Postgres(pool) => sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(pool)
            .await
            .map(|_| ()),
    }
}

async fn contract(db: &Database, id_type: &str, migration: &str, false_value: &str) {
    execute(db, &format!("CREATE TABLE jobs(id {id_type} PRIMARY KEY); CREATE TABLE documents(id {id_type} PRIMARY KEY); INSERT INTO documents VALUES ('00000000-0000-0000-0000-000000000001');")).await.unwrap();
    execute(db, migration).await.unwrap();
    let insert = "INSERT INTO processing_stage_runs(id, document_id, vault_id, job_id, group_id, stage, state, attempt, queue_attempt, created_at, updated_at) VALUES ('00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000004', 'ocr', 'pending', 0, 0, '2026-11-03T00:00:00Z', '2026-11-03T00:00:00Z')";
    execute(db, insert).await.unwrap();
    assert!(
        execute(
            db,
            "UPDATE processing_stage_runs SET state = 'running', attempt = 1"
        )
        .await
        .is_err(),
        "running requires a claimed queue attempt"
    );
    execute(db, "UPDATE processing_stage_runs SET state = 'running', attempt = 1, queue_attempt = 1, claim_id = '00000000-0000-0000-0000-000000000005'").await.unwrap();
    assert!(execute(db, "UPDATE processing_stage_runs SET state = 'waiting_to_retry', claim_id = NULL, reason_code = 'provider_failed'").await.is_err(), "retries require a schedule");
    execute(db, "UPDATE processing_stage_runs SET state = 'waiting_to_retry', claim_id = NULL, reason_code = 'provider_failed', retry_at = '2026-11-03T00:00:10Z'").await.unwrap();
    assert!(execute(db, "UPDATE processing_stage_runs SET state = 'failed', retry_at = NULL, reason_code = 'secret provider error'").await.is_err(), "raw errors are not stage reasons");
    assert!(execute(db, "UPDATE processing_stage_runs SET state = 'completed', retry_at = NULL, reason_code = NULL").await.is_err(), "OCR completion records whether text exists");
    execute(db, &format!("UPDATE processing_stage_runs SET state = 'completed', retry_at = NULL, reason_code = NULL, has_text = {false_value}")).await.unwrap();
    let new_run = insert
        .replace("000000000002", "000000000006")
        .replace("000000000004", "000000000007");
    assert!(
        execute(db, &new_run).await.is_err(),
        "only one current run per document stage"
    );
    execute(
        db,
        &format!("UPDATE processing_stage_runs SET is_current = {false_value}"),
    )
    .await
    .unwrap();
    execute(db, &new_run).await.unwrap();
    execute(
        db,
        &format!("UPDATE processing_stage_runs SET is_current = {false_value}"),
    )
    .await
    .unwrap();
    assert!(
        execute(db, &new_run.replace("000000000006", "000000000008"))
            .await
            .is_err(),
        "a queue retry cannot create another run"
    );
    let planned = insert
        .replace("000000000002", "000000000009")
        .replace(
            "'00000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000004'",
            "NULL, '00000000-0000-0000-0000-000000000004'",
        )
        .replace("'ocr'", "'embedding'");
    execute(db, &planned).await.unwrap();
    execute(
        db,
        &format!("UPDATE processing_stage_runs SET is_current = {false_value}"),
    )
    .await
    .unwrap();
    assert!(
        execute(db, &planned.replace("000000000009", "000000000010"))
            .await
            .is_err(),
        "an unassigned planned stage still has a unique group identity"
    );
    execute(db, "DELETE FROM documents").await.unwrap();
    let remaining: i64 = match db {
        Database::Sqlite(pool) => sqlx::query_scalar("SELECT COUNT(*) FROM processing_stage_runs")
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => {
            sqlx::query_scalar("SELECT COUNT(*) FROM processing_stage_runs")
                .fetch_one(pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn sqlite_stage_run_schema() {
    contract(
        &Database::connect("sqlite::memory:").await.unwrap(),
        "TEXT",
        include_str!("../../../migrations/sqlite/20261102170100_processing_stage_runs.sql"),
        "0",
    )
    .await;
}

#[tokio::test]
async fn postgres_stage_run_schema() {
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
    contract(
        &db,
        "UUID",
        include_str!("../../../migrations/postgres/20261102170100_processing_stage_runs.sql"),
        "FALSE",
    )
    .await;
}
