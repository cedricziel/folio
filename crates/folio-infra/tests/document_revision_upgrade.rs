mod common;

use std::borrow::Cow;

use bytes::Bytes;
use folio_core::models::{FileIdentity, Id, VaultId};
use folio_core::ports::DocumentStore;
use folio_core::services::backfill_original_identities;
use folio_infra::{Database, ObjectStoreDocumentStore};
use sqlx::migrate::Migrator;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::{runners::AsyncRunner, ImageExt};

static SQLITE: Migrator = sqlx::migrate!("../../migrations/sqlite");
static POSTGRES: Migrator = sqlx::migrate!("../../migrations/postgres");
const REVISION_MIGRATION: i64 = 20261122190000;

const SOURCE: &str = "00000000-0000-4000-8000-000000000001";
const DESTINATION: &str = "00000000-0000-4000-8000-000000000002";
const STORED: &str = "00000000-0000-4000-8000-000000000003";
const METADATA_ONLY: &str = "00000000-0000-4000-8000-000000000004";
const INCOMPLETE: &str = "00000000-0000-4000-8000-000000000005";

async fn revision_upgrade_contract(db: Database) {
    let all = match &db {
        Database::Sqlite(_) => &SQLITE,
        Database::Postgres(_) => &POSTGRES,
    };
    let legacy = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < REVISION_MIGRATION)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    match &db {
        Database::Sqlite(pool) => legacy.run(pool).await.unwrap(),
        Database::Postgres(pool) => legacy.run(pool).await.unwrap(),
    };

    macro_rules! execute {
        ($sql:expr) => {{
            let sql = sqlx::AssertSqlSafe($sql);
            match &db {
                Database::Sqlite(pool) => sqlx::raw_sql(sql).execute(pool).await.map(|_| ()),
                Database::Postgres(pool) => sqlx::raw_sql(sql).execute(pool).await.map(|_| ()),
            }
        }};
    }
    macro_rules! count {
        ($sql:expr) => {{
            let sql = sqlx::AssertSqlSafe($sql);
            match &db {
                Database::Sqlite(pool) => sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await,
                Database::Postgres(pool) => sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await,
            }
            .unwrap()
        }};
    }

    let time = "2001-02-03T04:05:06Z";
    execute!(format!(
        "INSERT INTO vaults(id, name, created_at, updated_at) VALUES
             ('{SOURCE}', 'Source', '{time}', '{time}'),
             ('{DESTINATION}', 'Destination', '{time}', '{time}');
         INSERT INTO documents(id, title, vault_id, file_path, upload_pending, created_at, updated_at) VALUES
             ('{STORED}', 'Stored', '{SOURCE}', 'old/original', FALSE, '{time}', '{time}'),
             ('{METADATA_ONLY}', 'Metadata only', '{SOURCE}', NULL, FALSE, '{time}', '{time}'),
             ('{INCOMPLETE}', 'Incomplete', '{SOURCE}', 'staged/original', TRUE, '{time}', '{time}');"
    ))
    .unwrap();

    db.migrate().await.unwrap();

    assert_eq!(
        count!(format!(
            "SELECT COUNT(*) FROM documents d
             JOIN document_revisions r ON r.id = d.current_revision_id AND r.document_id = d.id
             JOIN document_files f ON f.id = r.original_file_id AND f.document_id = d.id
             WHERE d.id = '{STORED}' AND r.vault_id = '{SOURCE}' AND f.vault_id = '{SOURCE}'
               AND r.number = 1 AND r.created_at = d.created_at AND d.updated_at = '{time}'
               AND f.storage_key = 'old/original' AND f.verification_state = 'pending'
               AND f.sha256 IS NULL AND f.byte_size IS NULL
               AND f.mime_type = 'application/octet-stream'"
        )),
        1,
        "the stored file becomes revision one with its legacy key and unverified identity"
    );
    assert_eq!(
        count!("SELECT COUNT(*) FROM document_revisions".to_owned()),
        1
    );
    assert_eq!(count!("SELECT COUNT(*) FROM outbox_events".to_owned()), 0);

    let repositories = db.repositories().await;
    let selected = repositories
        .documents
        .list_unverified_originals(None, 1)
        .await
        .unwrap();
    let original = &selected[0];
    execute!(format!(
        "UPDATE documents SET vault_id = '{DESTINATION}' WHERE id = '{STORED}'"
    ))
    .unwrap();
    assert!(
        !repositories
            .documents
            .record_original_verification(original, None)
            .await
            .unwrap(),
        "an earlier storage read cannot update a file after its vault changes"
    );
    execute!(format!(
        "UPDATE documents SET vault_id = '{SOURCE}' WHERE id = '{STORED}'"
    ))
    .unwrap();
    let store = ObjectStoreDocumentStore::in_memory();
    let backfill = || backfill_original_identities(repositories.documents.as_ref(), &store, 1);
    assert_eq!(backfill().await.unwrap(), 0);
    assert_eq!(
        count!(
            "SELECT COUNT(*) FROM document_files WHERE verification_state = 'unavailable'
                AND sha256 IS NULL AND byte_size IS NULL"
                .to_owned()
        ),
        1,
        "a missing original stays retryable without an invented identity"
    );
    let original = b"legacy original bytes";
    store
        .put("old/original", Bytes::from_static(original))
        .await
        .unwrap();
    assert_eq!(backfill().await.unwrap(), 1);
    let identity = FileIdentity::from_bytes("old/original", "application/octet-stream", original);
    assert_eq!(
        count!(format!(
            "SELECT COUNT(*) FROM document_files WHERE verification_state = 'verified'
               AND sha256 = '{}' AND byte_size = {}",
            identity.sha256, identity.byte_size
        )),
        1
    );
    assert_eq!(backfill().await.unwrap(), 0, "verified files are skipped");
    assert_eq!(count!("SELECT COUNT(*) FROM outbox_events".to_owned()), 0);

    let result = execute!(format!(
        "UPDATE documents SET current_revision_id = (
             SELECT id FROM document_revisions WHERE document_id = '{STORED}'
         ) WHERE id = '{METADATA_ONLY}'"
    ));
    assert!(
        result.is_err(),
        "a revision must belong to its current document"
    );

    execute!(format!(
        "UPDATE documents SET vault_id = '{DESTINATION}' WHERE id = '{STORED}'"
    ))
    .unwrap();
    assert_eq!(
        count!(format!(
            "SELECT COUNT(*) FROM document_revisions r JOIN document_files f ON f.id = r.original_file_id
             WHERE r.document_id = '{STORED}' AND r.vault_id = '{DESTINATION}' AND f.vault_id = '{DESTINATION}'"
        )),
        1,
        "moving a document transfers ownership of its history"
    );

    execute!(format!("DELETE FROM documents WHERE id = '{STORED}'")).unwrap();
    assert_eq!(
        count!("SELECT COUNT(*) FROM document_revisions".to_owned()),
        0
    );
    assert_eq!(count!("SELECT COUNT(*) FROM document_files".to_owned()), 0);
    let vault = VaultId(SOURCE.parse().unwrap());
    let incomplete: Id = INCOMPLETE.parse().unwrap();
    let original = FileIdentity::from_bytes("staged/original", "application/pdf", b"published");
    assert!(repositories
        .documents
        .finalize_verified_upload(vault, incomplete, &original)
        .await
        .unwrap());
    assert!(!repositories
        .documents
        .finalize_verified_upload(vault, incomplete, &original)
        .await
        .unwrap());
    let changed = FileIdentity::from_bytes("staged/original", "application/pdf", b"different");
    assert!(matches!(
        repositories
            .documents
            .finalize_verified_upload(vault, incomplete, &changed)
            .await,
        Err(folio_core::Error::Conflict(_))
    ));
    assert!(matches!(
        repositories
            .documents
            .set_file_path(vault, incomplete, "other")
            .await,
        Err(folio_core::Error::Conflict(_))
    ));
    assert!(matches!(
        repositories
            .documents
            .finalize_upload(vault, incomplete, "other", 42)
            .await,
        Err(folio_core::Error::Conflict(_))
    ));
    assert_eq!(
        count!("SELECT COUNT(*) FROM document_revisions WHERE number = 1".to_owned()),
        1
    );
    assert_eq!(
        count!(
            "SELECT COUNT(*) FROM outbox_events WHERE type = 'document.revision_created'"
                .to_owned()
        ),
        1
    );
}

#[tokio::test]
async fn sqlite_upgrade_preserves_legacy_original_and_document_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::connect(&format!(
        "sqlite:{}?mode=rwc",
        dir.path().join("revision-upgrade.db").display()
    ))
    .await
    .unwrap();
    revision_upgrade_contract(db).await;
}

#[tokio::test]
async fn postgres_upgrade_preserves_legacy_original_and_document_ownership() {
    let Some(container) = common::container_or_skip(
        "document revisions",
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
    revision_upgrade_contract(db).await;
}
