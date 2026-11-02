mod common;

use std::borrow::Cow;

use chrono::{Duration, Utc};
use folio_core::events::decode_outbox_payload;
use folio_core::models::{Id, UserId, VaultRole};
use folio_core::{DocumentEvent, EventOrigin};
use folio_infra::Database;
use serde_json::{json, Value};
use sqlx::migrate::Migrator;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ImageExt;

const VAULT_VERSION: i64 = 20261102010000;
const OUTBOX_VERSION: i64 = 20261102020000;
const SNAPSHOT_VERSION: i64 = 20261102040000;
const DOCUMENT_ID: &str = "10000000-0000-4000-8000-000000000001";
const TAG_ID: &str = "10000000-0000-4000-8000-000000000002";
static SQLITE_MIGRATIONS: Migrator = sqlx::migrate!("../../migrations/sqlite");
static POSTGRES_MIGRATIONS: Migrator = sqlx::migrate!("../../migrations/postgres");

fn migrations(db: &Database) -> &'static Migrator {
    match db {
        Database::Sqlite(_) => &SQLITE_MIGRATIONS,
        Database::Postgres(_) => &POSTGRES_MIGRATIONS,
    }
}

async fn legacy_schema(db: &Database) {
    let legacy = Migrator {
        migrations: Cow::Owned(
            migrations(db)
                .iter()
                .filter(|m| m.version <= OUTBOX_VERSION && m.version != VAULT_VERSION)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    match db {
        Database::Sqlite(pool) => legacy.run(pool).await.unwrap(),
        Database::Postgres(pool) => legacy.run(pool).await.unwrap(),
    }
}

async fn apply_vault_schema(db: &Database) {
    match db {
        Database::Sqlite(pool) => migrations(db).run_to(VAULT_VERSION, pool).await.unwrap(),
        Database::Postgres(pool) => migrations(db).run_to(VAULT_VERSION, pool).await.unwrap(),
    }
}

async fn execute(db: &Database, sql: &str) {
    match db {
        Database::Sqlite(pool) => {
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                .execute(pool)
                .await
                .unwrap();
        }
        Database::Postgres(pool) => {
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

async fn scalar(db: &Database, sql: &str) -> i64 {
    match db {
        Database::Sqlite(pool) => sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(pool)
            .await
            .unwrap(),
    }
}

fn deletion_snapshot() -> Value {
    json!({
        "event": {"type": "deleted", "document_id": DOCUMENT_ID},
        "origin": "user",
        "snapshot": {
            "document": {
                "id": DOCUMENT_ID,
                "title": "Deleted before vaults",
                "content": "Retained workflow content",
                "original_filename": "deleted.pdf",
                "file_path": "documents/deleted/original",
                "mime_type": "application/pdf",
                "correspondent_id": null,
                "document_type_id": null,
                "page_count": 2,
                "source": {"kind": "upload"},
                "ocr_status": "done",
                "embed_status": "done",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            },
            "tags": [{
                "id": TAG_ID,
                "name": "Retained tag",
                "color": "#123456",
                "created_at": "2026-01-01T00:00:00Z"
            }]
        }
    })
}

async fn insert_payload(db: &Database, kind: &str, payload: &str) -> i64 {
    match db {
        Database::Sqlite(pool) => {
            sqlx::query_scalar(
                "INSERT INTO outbox_events (type, payload, created_at, available_at)
                 VALUES (?1, ?2, '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z') RETURNING id",
            )
            .bind(kind)
            .bind(payload)
            .fetch_one(pool)
            .await
            .unwrap()
        }
        Database::Postgres(pool) => {
            sqlx::query_scalar(
                "INSERT INTO outbox_events (type, payload, created_at, available_at)
                 VALUES ($1, $2::jsonb, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z') RETURNING id",
            )
            .bind(kind)
            .bind(payload)
            .fetch_one(pool)
            .await
            .unwrap()
        }
    }
}

async fn payload(db: &Database, id: i64) -> String {
    match db {
        Database::Sqlite(pool) => {
            sqlx::query_scalar("SELECT payload FROM outbox_events WHERE id = ?1")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap()
        }
        Database::Postgres(pool) => {
            sqlx::query_scalar("SELECT payload::text FROM outbox_events WHERE id = $1")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap()
        }
    }
}

async fn snapshot_upgrade_contract(db: &Database, with_live_document: bool) {
    legacy_schema(db).await;
    assert_eq!(
        scalar(db, "SELECT MAX(version) FROM _sqlx_migrations").await,
        OUTBOX_VERSION
    );
    execute(db, "INSERT INTO users (id, username, username_normalized, password_hash, created_at, is_admin, is_owner)
        VALUES ('old-owner', 'old-owner', 'old-owner', '!', 1, TRUE, TRUE),
               ('later-admin', 'later-admin', 'later-admin', '!', 2, TRUE, FALSE)").await;
    if with_live_document {
        execute(db, "INSERT INTO documents (id, title, created_at, updated_at)
            VALUES ('10000000-0000-4000-8000-000000000003', 'Still alive', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')").await;
    }
    let legacy = deletion_snapshot();
    assert!(decode_outbox_payload(&legacy.to_string())
        .unwrap_err()
        .to_string()
        .contains("vault_id"));
    let old_id = insert_payload(db, "deleted", &legacy.to_string()).await;
    let corrupt_id = if matches!(db, Database::Sqlite(_)) {
        Some(insert_payload(db, "deleted", "{corrupt legacy payload").await)
    } else {
        None
    };
    apply_vault_schema(db).await;
    let repositories = db.repositories().await;
    let current_vault = repositories
        .vaults
        .create("Current vault", None)
        .await
        .unwrap()
        .id;
    let mut current = deletion_snapshot();
    current["snapshot"]["document"]["vault_id"] = json!(current_vault);
    current["snapshot"]["document"]["uploaded_by"] = json!("known-uploader");
    let current_id = insert_payload(db, "deleted", &current.to_string()).await;

    db.migrate().await.unwrap();

    let updated = payload(db, old_id).await;
    let (event, origin) = decode_outbox_payload(&updated)
        .expect("a pre-vault deletion remains decodable after upgrade");
    let DocumentEvent::Deleted {
        document_id,
        snapshot,
    } = event
    else {
        panic!("expected the retained deletion event");
    };
    let snapshot = snapshot.unwrap();
    let archive = snapshot.document.vault_id;
    assert_eq!(document_id, DOCUMENT_ID.parse::<Id>().unwrap());
    assert_eq!(origin, EventOrigin::User);
    assert_eq!(snapshot.document.uploaded_by, None);
    assert_eq!(
        snapshot.document.content.as_deref(),
        Some("Retained workflow content")
    );
    assert_eq!(snapshot.tags.len(), 1);
    assert_eq!(snapshot.tags[0].id, TAG_ID.parse::<Id>().unwrap());
    assert_eq!(snapshot.tags[0].name, "Retained tag");
    assert_eq!(snapshot.tags[0].color.as_deref(), Some("#123456"));
    assert_eq!(
        repositories
            .vaults
            .find(archive)
            .await
            .unwrap()
            .unwrap()
            .name,
        "Archive"
    );
    assert_eq!(
        repositories
            .members
            .direct_owner_count(archive)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        repositories
            .access
            .effective_role(&UserId::from("old-owner"), archive)
            .await
            .unwrap(),
        Some(VaultRole::Owner)
    );
    assert_eq!(
        repositories
            .access
            .effective_role(&UserId::from("later-admin"), archive)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        repositories
            .vaults
            .default_for_user(&UserId::from("old-owner"))
            .await
            .unwrap(),
        Some(archive)
    );
    let preserved: Value = serde_json::from_str(&payload(db, current_id).await).unwrap();
    assert_eq!(preserved, current);
    assert_eq!(
        scalar(db, "SELECT COUNT(*) FROM vaults WHERE name = 'Archive'").await,
        1
    );
    assert_eq!(scalar(db, &format!("SELECT COUNT(*) FROM _sqlx_migrations WHERE version IN ({VAULT_VERSION}, {SNAPSHOT_VERSION})")).await, 2);
    if with_live_document {
        assert_eq!(
            repositories
                .documents
                .locate("10000000-0000-4000-8000-000000000003".parse().unwrap())
                .await
                .unwrap(),
            Some(archive)
        );
    } else {
        assert!(repositories.documents.list_all().await.unwrap().is_empty());
        assert!(repositories.tags.list(archive).await.unwrap().is_empty());
    }
    if let Some(id) = corrupt_id {
        assert_eq!(payload(db, id).await, "{corrupt legacy payload");
    }
    let replayed = repositories
        .outbox
        .replay(0, chrono::DateTime::<Utc>::UNIX_EPOCH, 10)
        .await
        .unwrap();
    assert_eq!(
        replayed.iter().map(|e| e.id).collect::<Vec<_>>(),
        [old_id, current_id]
    );
    let claimed = repositories
        .outbox
        .claim(10, Duration::seconds(30), 10)
        .await
        .unwrap();
    assert_eq!(
        claimed.iter().map(|e| e.id).collect::<Vec<_>>(),
        [old_id, current_id]
    );
    assert_eq!(claimed[0].attempts, 1);
    assert!(matches!(
        &claimed[0].event,
        DocumentEvent::Deleted {
            snapshot: Some(_),
            ..
        }
    ));
    db.migrate().await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&payload(db, old_id).await).unwrap(),
        serde_json::from_str::<Value>(&updated).unwrap()
    );
}

async fn empty_upgrade_contract(db: &Database) {
    legacy_schema(db).await;
    let plain = json!({"event": {"type": "created", "document_id": DOCUMENT_ID}, "origin": "user", "snapshot": null});
    let id = insert_payload(db, "created", &plain.to_string()).await;

    db.migrate().await.unwrap();

    assert_eq!(scalar(db, "SELECT COUNT(*) FROM vaults").await, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&payload(db, id).await).unwrap(),
        plain
    );
}

async fn sqlite_snapshot_fixture(with_live_document: bool) {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("snapshot.db").display()
    ))
    .await
    .unwrap();
    snapshot_upgrade_contract(&db, with_live_document).await;
}

#[tokio::test]
async fn sqlite_snapshot_only_upgrade_preserves_pending_deletions() {
    sqlite_snapshot_fixture(false).await;
}

#[tokio::test]
async fn sqlite_snapshot_upgrade_reuses_the_live_documents_archive() {
    sqlite_snapshot_fixture(true).await;
}

#[tokio::test]
async fn sqlite_non_snapshot_events_do_not_create_an_archive() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("empty.db").display()
    ))
    .await
    .unwrap();
    empty_upgrade_contract(&db).await;
}

#[tokio::test]
async fn postgres_snapshot_upgrade_preserves_pending_deletions_and_empty_installs() {
    let Some(container) = common::container_or_skip(
        "PostgreSQL snapshot upgrade contract",
        Postgres::default().with_tag("17-alpine").start().await,
    ) else {
        return;
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let admin = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    execute(&admin, "CREATE DATABASE snapshot_only").await;
    execute(&admin, "CREATE DATABASE snapshot_live").await;
    execute(&admin, "CREATE DATABASE empty_events").await;
    let only = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/snapshot_only"
    ))
    .await
    .unwrap();
    snapshot_upgrade_contract(&only, false).await;
    let live = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/snapshot_live"
    ))
    .await
    .unwrap();
    snapshot_upgrade_contract(&live, true).await;
    let empty = Database::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/empty_events"
    ))
    .await
    .unwrap();
    empty_upgrade_contract(&empty).await;
}
