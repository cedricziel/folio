//! SQLite adapters
mod metadata_option_repo;
pub use metadata_option_repo::SqliteMetadataOptionRepository;

mod metadata_repo;
pub use metadata_repo::SqliteMetadataFieldRepository;

mod auth_repo;
mod document_entries;
mod document_repo;
mod ingest_outcomes;
mod ingest_repo;
mod outbox;
mod search;
mod settings_repo;
mod vault_repo;
mod workflow_repo;

use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{ConnectOptions, Connection, SqlitePool};

pub use auth_repo::SqliteAuthStore;
pub use document_entries::SqliteDocumentEntryRepository;
pub use document_repo::{
    SqliteCorrespondentRepository, SqliteDocumentRepository, SqliteDocumentTypeRepository,
    SqliteTagRepository,
};
pub use ingest_outcomes::SqliteIngestOutcomeRepository;
pub use ingest_repo::{SqliteIngestLedger, SqliteSourceRepository};
pub(crate) use outbox::capture_document;
pub use outbox::{
    insert_domain_event, insert_domain_event_with_origin, insert_event_with_envelope,
    SqliteOutboxStore,
};
pub use search::SqliteSearchService;
pub use settings_repo::SqliteSettingsRepository;
pub use vault_repo::SqliteVaultRepository;
pub use workflow_repo::SqliteWorkflowRepository;

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
    let pool_options = if memory {
        SqlitePoolOptions::new()
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
    } else {
        SqlitePoolOptions::new().max_connections(5)
    };
    pool_options.connect_with(options).await
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
