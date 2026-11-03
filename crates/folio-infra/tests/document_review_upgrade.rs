mod common;

use std::borrow::Cow;

use chrono::{DateTime, Utc};
use folio_core::models::{CreateDocumentType, Id};
use folio_infra::Database;
use sqlx::migrate::Migrator;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

static SQLITE: Migrator = sqlx::migrate!("../../migrations/sqlite");
static POSTGRES: Migrator = sqlx::migrate!("../../migrations/postgres");

async fn upgrade_contract(db: Database) {
    let all = match &db {
        Database::Sqlite(_) => &SQLITE,
        Database::Postgres(_) => &POSTGRES,
    };
    let legacy = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 20261102161030)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    match &db {
        Database::Sqlite(pool) => legacy.run(pool).await.unwrap(),
        Database::Postgres(pool) => legacy.run(pool).await.unwrap(),
    }
    let r = db.repositories().await;
    let vault = r.vaults.instance_default().await.unwrap();
    let kind = r
        .document_types
        .create(
            vault,
            CreateDocumentType {
                name: "Legacy classification".into(),
            },
        )
        .await
        .unwrap();
    let document_id = Id::new_v4();
    let imported: DateTime<Utc> = "2001-02-03T04:05:06Z".parse().unwrap();
    let insert = "INSERT INTO documents(id,vault_id,title,ocr_status,embed_status,document_type_id,created_at,updated_at) VALUES ($1,$2,'Legacy file','done','pending',$3,$4,$4)";
    match &db {
        Database::Sqlite(pool) => {
            sqlx::query(insert)
                .bind(document_id.to_string())
                .bind(vault.to_string())
                .bind(kind.id.to_string())
                .bind(imported.to_rfc3339())
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::query(insert)
                .bind(document_id)
                .bind(vault.0)
                .bind(kind.id)
                .bind(imported)
                .execute(pool)
                .await
                .unwrap();
        }
    }
    db.migrate().await.unwrap();
    let count = "SELECT COUNT(*) FROM documents WHERE reviewed_at IS NOT NULL AND reviewed_by IS NULL AND review_origin = 'legacy_classification'";
    let legacy_count: i64 = match &db {
        Database::Sqlite(pool) => sqlx::query_scalar(count).fetch_one(pool).await.unwrap(),
        Database::Postgres(pool) => sqlx::query_scalar(count).fetch_one(pool).await.unwrap(),
    };
    assert_eq!(legacy_count, 1);
    let doc = r
        .documents
        .find_by_id(vault, document_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(doc.created_at, imported);
    assert_eq!(doc.updated_at, imported);
    let review_events = "SELECT COUNT(*) FROM outbox_events WHERE type IN ('document.reviewed', 'document.unreviewed')";
    let event_count: i64 = match &db {
        Database::Sqlite(pool) => sqlx::query_scalar(review_events)
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => sqlx::query_scalar(review_events)
            .fetch_one(pool)
            .await
            .unwrap(),
    };
    assert_eq!(event_count, 0);
}

#[tokio::test]
async fn sqlite_legacy_review_records_migration_provenance_without_events() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("upgrade.db").display()
    ))
    .await
    .unwrap();
    upgrade_contract(db).await;
}

#[tokio::test]
async fn postgres_legacy_review_records_migration_provenance_without_events() {
    let Some(container) = common::container_or_skip(
        "document review",
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
    upgrade_contract(db).await;
}
