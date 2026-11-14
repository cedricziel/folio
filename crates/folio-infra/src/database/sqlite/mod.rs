//! SQLite adapters
mod metadata_option_repo;
pub use metadata_option_repo::SqliteMetadataOptionRepository;

mod metadata_repo;
pub use metadata_repo::SqliteMetadataFieldRepository;

mod metadata_value_repo;
pub use metadata_value_repo::SqliteMetadataValueRepository;
pub(crate) use metadata_value_repo::{clear_incoming_links, prepare_move};

mod auth_repo;
mod document_entries;
mod document_repo;
mod import_runs;
mod ingest_outcomes;
mod ingest_repo;
mod outbox;
mod push_repo;
mod search;
mod settings_repo;
mod upload_receipts;
mod vault_repo;
mod workflow_repo;

use std::str::FromStr;

use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{ConnectOptions, Connection, SqlitePool};

pub use auth_repo::SqliteAuthStore;
pub use document_entries::SqliteDocumentEntryRepository;
pub use document_repo::{
    SqliteCorrespondentRepository, SqliteDocumentRepository, SqliteDocumentTypeRepository,
    SqliteTagRepository,
};
pub use import_runs::{
    put_paperless_sync_state, record_import_applied, transition_import_run,
    upsert_paperless_document, SqliteImportRepository,
};
pub use ingest_outcomes::SqliteIngestOutcomeRepository;
pub use ingest_repo::{SqliteIngestLedger, SqliteSourceRepository};
pub(crate) use outbox::capture_document;
pub use outbox::{
    insert_domain_event, insert_domain_event_with_origin, insert_event_with_envelope,
    SqliteOutboxStore,
};
pub use push_repo::SqlitePushRepository;
pub use search::SqliteSearchService;
pub use settings_repo::SqliteSettingsRepository;
pub use upload_receipts::SqliteUploadReceiptRepository;
pub use vault_repo::SqliteVaultRepository;
pub use workflow_repo::SqliteWorkflowRepository;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Create a new SQLite connection pool
pub async fn create_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .foreign_keys(true);

    // SQLx cannot serialize its generated in-memory filename into a URL.
    let memory = options
        .clone()
        .filename("folio")
        .to_url_lossy()
        .query_pairs()
        .any(|(key, value)| key == "mode" && value == "memory");
    if memory {
        return SqlitePoolOptions::new()
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(options)
            .await;
    }
    // WAL lets readers and the single writer proceed together. NORMAL is safe
    // in WAL: a crash cannot corrupt the file, only the last commits can be
    // lost on power failure. Writers queue for up to the busy timeout.
    let options = options
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(BUSY_TIMEOUT);
    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
}

/// Run SQLite database migrations
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    let mut connection = pool.acquire().await?;
    let file_backed: bool =
        sqlx::query_scalar("SELECT file != '' FROM pragma_database_list WHERE name = 'main'")
            .fetch_one(&mut *connection)
            .await?;
    if file_backed {
        // A connection lock permits the vault migration's explicit transactions.
        connection.close_on_drop();
        sqlx::raw_sql("BEGIN EXCLUSIVE; PRAGMA locking_mode = EXCLUSIVE; COMMIT;")
            .execute(&mut *connection)
            .await?;
    }
    sqlx::migrate!("../../migrations/sqlite")
        .run_direct(None, &mut *connection, false)
        .await?;
    let pending: bool =
        sqlx::query_scalar("SELECT pending FROM pipeline_initialization WHERE id = 1")
            .fetch_one(&mut *connection)
            .await?;
    if pending {
        let mut transaction = connection.begin().await?;
        sqlx::query(super::CLEAR_PIPELINE_DEFAULTS)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE pipeline_initialization SET pending = false WHERE id = 1")
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
    }
    if file_backed {
        connection.close().await?;
    }
    Ok(())
}

mod saved_view_repo;
pub use saved_view_repo::SqliteSavedViewRepository;

#[cfg(test)]
mod pool_tests {
    use super::*;

    async fn file_pool() -> (SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite:{}?mode=rwc", dir.path().join("pool.db").display());
        (create_pool(&url).await.unwrap(), dir)
    }

    #[tokio::test]
    async fn file_database_uses_wal_with_busy_timeout() {
        let (pool, _dir) = file_pool().await;
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        let timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(&pool)
            .await
            .unwrap();
        let sync: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(timeout, 5000);
        assert_eq!(sync, 1, "NORMAL");
    }

    #[tokio::test]
    async fn second_writer_waits_for_the_first() {
        let (pool, _dir) = file_pool().await;
        sqlx::query("CREATE TABLE t (v INTEGER)")
            .execute(&pool)
            .await
            .unwrap();
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO t VALUES (1)")
            .execute(&mut *tx)
            .await
            .unwrap();
        let other = pool.clone();
        let waiting = tokio::spawn(async move {
            sqlx::query("INSERT INTO t VALUES (2)")
                .execute(&other)
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!waiting.is_finished(), "second write must wait, not fail");
        tx.commit().await.unwrap();
        waiting.await.unwrap().unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM t")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 2);
    }

    #[tokio::test]
    async fn memory_database_still_works() {
        let pool = create_pool("sqlite::memory:").await.unwrap();
        let one: i64 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(one, 1);
    }
}
