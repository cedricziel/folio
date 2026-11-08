mod common;

use std::borrow::Cow;

use folio_infra::Database;
use sqlx::migrate::Migrator;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

static SQLITE: Migrator = sqlx::migrate!("../../migrations/sqlite");
static POSTGRES: Migrator = sqlx::migrate!("../../migrations/postgres");

const PAPERLESS_RUNS: i64 = 20261107130000;

async fn exec(db: &Database, sql: &str) -> Result<u64, sqlx::Error> {
    match db {
        Database::Sqlite(pool) => Ok(sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(pool)
            .await?
            .rows_affected()),
        Database::Postgres(pool) => Ok(sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(pool)
            .await?
            .rows_affected()),
    }
}

async fn count(db: &Database, sql: &str) -> i64 {
    match db {
        Database::Sqlite(pool) => sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_one(pool)
            .await
            .unwrap(),
    }
}

fn source_row(id: &str, vault: &str, name: &str, kind: &str, deleted: bool) -> String {
    let deleted_at = if deleted {
        "'2026-01-02T00:00:00Z'"
    } else {
        "NULL"
    };
    format!(
        "INSERT INTO ingestion_sources (id, vault_id, name, enabled, kind, config, created_at, updated_at, deleted_at) \
         VALUES ('{id}', '{vault}', '{name}', TRUE, '{kind}', '{{}}', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', {deleted_at})"
    )
}

async fn contract(db: Database) {
    let all = match &db {
        Database::Sqlite(_) => &SQLITE,
        Database::Postgres(_) => &POSTGRES,
    };
    let legacy = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < PAPERLESS_RUNS)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    match &db {
        Database::Sqlite(pool) => legacy.run(pool).await.unwrap(),
        Database::Postgres(pool) => legacy.run(pool).await.unwrap(),
    }
    let vault = common::legacy_vault(&db).await.to_string();
    let other_vault = common::legacy_vault(&db).await.to_string();
    let source = "00000000-0000-4000-8000-000000000001";
    let item = "00000000-0000-4000-8000-000000000002";
    let attempt = "00000000-0000-4000-8000-000000000003";
    let event = "00000000-0000-4000-8000-000000000004";
    let locator = "a".repeat(64);
    exec(&db, &format!(
        "INSERT INTO ingest_items (id, vault_id, source_id, source_managed, source_kind, locator_key, attempt_id, started_at, lease_expires_at, decision_fingerprint, state) \
         VALUES ('{item}', '{vault}', '{source}', TRUE, 'folder', '{locator}', '{attempt}', '2026-01-01T00:00:00Z', '2026-01-01T00:05:00Z', '{locator}', 'skipped')"
    ))
    .await
    .unwrap();
    exec(&db, &format!(
        "INSERT INTO ingest_outcomes (vault_id, source_id, item_id, attempt_id, event_id, occurred_at, outcome) \
         VALUES ('{vault}', '{source}', '{item}', '{attempt}', '{event}', '2026-01-01T00:00:00Z', 'skipped')"
    ))
    .await
    .unwrap();
    exec(&db, &format!(
        "INSERT INTO source_health (vault_id, source_id, source_managed, source_kind, failed) VALUES ('{vault}', '{source}', TRUE, 'email', FALSE)"
    ))
    .await
    .unwrap();

    db.migrate().await.unwrap();

    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM ingest_items WHERE state = 'skipped' AND source_kind = 'folder'"
        )
        .await,
        1,
        "ingest items survive the rebuild"
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM ingest_outcomes WHERE outcome = 'skipped'"
        )
        .await,
        1,
        "ingest outcomes survive the rebuild"
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM source_health WHERE source_kind = 'email'"
        )
        .await,
        1,
        "source health survives the rebuild"
    );
    exec(&db, "DELETE FROM ingest_items").await.unwrap();
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM ingest_outcomes").await,
        0,
        "outcomes still cascade from their item"
    );

    let paperless_source = "00000000-0000-4000-8000-000000000010";
    exec(&db, &format!(
        "INSERT INTO source_health (vault_id, source_id, source_managed, source_kind, failed) VALUES ('{vault}', '{paperless_source}', TRUE, 'paperless', FALSE)"
    ))
    .await
    .expect("source_health accepts paperless");
    exec(&db, &format!(
        "INSERT INTO ingest_items (id, vault_id, source_id, source_managed, source_kind, locator_key, attempt_id, started_at, lease_expires_at, decision_fingerprint, state) \
         VALUES ('{item}', '{vault}', '{paperless_source}', TRUE, 'paperless', '{locator}', '{attempt}', '2026-01-01T00:00:00Z', '2026-01-01T00:05:00Z', '{locator}', 'skipped')"
    ))
    .await
    .expect("ingest_items accepts paperless");
    assert!(
        exec(&db, &format!(
            "INSERT INTO source_health (vault_id, source_id, source_managed, source_kind, failed) VALUES ('{vault}', '{item}', TRUE, 'ftp', FALSE)"
        ))
        .await
        .is_err(),
        "unknown kinds stay rejected"
    );

    exec(
        &db,
        &source_row(paperless_source, &vault, "Paperless", "paperless", false),
    )
    .await
    .unwrap();
    let second = exec(
        &db,
        &source_row(
            "00000000-0000-4000-8000-000000000011",
            &vault,
            "Paperless 2",
            "paperless",
            false,
        ),
    )
    .await;
    assert!(
        second.is_err(),
        "a second live paperless source is rejected"
    );
    exec(
        &db,
        &source_row(
            "00000000-0000-4000-8000-000000000012",
            &vault,
            "Old Paperless",
            "paperless",
            true,
        ),
    )
    .await
    .expect("a deleted paperless source doesn't count");
    exec(
        &db,
        &source_row(
            "00000000-0000-4000-8000-000000000013",
            &other_vault,
            "Paperless",
            "paperless",
            false,
        ),
    )
    .await
    .expect("another vault has its own paperless source");
    exec(
        &db,
        &source_row(
            "00000000-0000-4000-8000-000000000014",
            &vault,
            "Folder",
            "folder",
            false,
        ),
    )
    .await
    .expect("other kinds are unaffected");

    for table in [
        "import_runs",
        "import_items",
        "import_staging",
        "paperless_documents",
        "paperless_taxonomy",
        "paperless_sync_state",
    ] {
        assert_eq!(
            count(&db, &format!("SELECT COUNT(*) FROM {table}")).await,
            0,
            "{table} exists"
        );
    }
}

#[tokio::test]
async fn sqlite_paperless_runs_schema() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("schema.db").display()
    ))
    .await
    .unwrap();
    contract(db).await;
}

#[tokio::test]
async fn postgres_paperless_runs_schema() {
    let Some(container) = common::container_or_skip(
        "paperless runs schema",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        container.get_host_port_ipv4(5432).await.unwrap()
    ))
    .await
    .unwrap();
    contract(db).await;
}
