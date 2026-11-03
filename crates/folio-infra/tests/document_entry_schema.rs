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

async fn count(db: &Database, sql: &'static str) -> i64 {
    match db {
        Database::Sqlite(pool) => sqlx::query_scalar(sql).fetch_one(pool).await.unwrap(),
        Database::Postgres(pool) => sqlx::query_scalar(sql).fetch_one(pool).await.unwrap(),
    }
}

async fn schema_contract(db: &Database, id_type: &str, migration: &str) {
    execute(db, &format!(
        "CREATE TABLE vaults(id {id_type} PRIMARY KEY);
         CREATE TABLE documents(id {id_type} PRIMARY KEY, vault_id {id_type} NOT NULL REFERENCES vaults(id));
         INSERT INTO vaults VALUES ('00000000-0000-0000-0000-000000000001'), ('00000000-0000-0000-0000-000000000002');
         INSERT INTO documents VALUES ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000001');"
    )).await.unwrap();
    execute(db, migration).await.unwrap();
    let insert = "INSERT INTO document_entries (id, document_id, vault_id, kind, body, created_at, updated_at)
        VALUES ('00000000-0000-0000-0000-000000000004', '00000000-0000-0000-0000-000000000003',
        '00000000-0000-0000-0000-000000000002', 'note', 'Text', '2026-11-02T00:00:00Z', '2026-11-02T00:00:00Z')";
    assert!(
        execute(db, insert).await.is_err(),
        "cross-vault entry must fail"
    );
    execute(db, &insert.replace("000000000002", "000000000001"))
        .await
        .unwrap();
    let receipt = "INSERT INTO document_entry_creation_receipts
        (vault_id, document_id, actor_scope, idempotency_key, request_fingerprint, entry_id, original_response, created_at)
        VALUES ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000003',
        'system', '00000000-0000-0000-0000-000000000005', 'fingerprint', '00000000-0000-0000-0000-000000000004',
        '{}', '2026-11-02T00:00:00Z')";
    execute(db, receipt).await.unwrap();
    assert!(
        execute(db, receipt).await.is_err(),
        "creation receipt must be unique even without a user"
    );
    execute(
        db,
        "UPDATE documents SET vault_id = '00000000-0000-0000-0000-000000000002'",
    )
    .await
    .unwrap();
    assert_eq!(count(db, "SELECT count(*) FROM document_entries WHERE vault_id = '00000000-0000-0000-0000-000000000002'").await, 1);
    assert_eq!(count(db, "SELECT count(*) FROM document_entry_creation_receipts WHERE vault_id = '00000000-0000-0000-0000-000000000002'").await, 1);
    execute(
        db,
        "UPDATE document_entries SET deleted_at = '2026-11-02T01:00:00Z'",
    )
    .await
    .unwrap();
    assert_eq!(
        count(db, "SELECT count(*) FROM document_entry_creation_receipts").await,
        1
    );
    execute(db, "DELETE FROM documents").await.unwrap();
    assert_eq!(count(db, "SELECT count(*) FROM document_entries").await, 0);
    assert_eq!(
        count(db, "SELECT count(*) FROM document_entry_creation_receipts").await,
        0
    );
    execute(db, "DROP TABLE document_entry_creation_receipts; DROP TABLE document_entries; DROP TABLE documents; DROP TABLE vaults;").await.unwrap();
    db.migrate().await.unwrap();
    assert_eq!(count(db, "SELECT count(*) FROM document_entries").await, 0);
}

#[tokio::test]
async fn sqlite_entry_schema_enforces_ownership_and_receipt_lifecycle() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    schema_contract(
        &db,
        "TEXT",
        include_str!("../../../migrations/sqlite/20261102163000_document_discussion.sql"),
    )
    .await;
}

#[tokio::test]
async fn postgres_entry_schema_enforces_ownership_and_receipt_lifecycle() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db = Database::connect(&format!(
        "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
    ))
    .await
    .unwrap();
    schema_contract(
        &db,
        "UUID",
        include_str!("../../../migrations/postgres/20261102163000_document_discussion.sql"),
    )
    .await;
}
