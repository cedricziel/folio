//! SQLite adapters

mod auth_repo;
mod document_repo;
mod ingest_repo;
mod outbox;
mod search;
mod settings_repo;
mod vault_repo;
mod workflow_repo;

use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

pub use auth_repo::SqliteAuthStore;
pub use document_repo::{
    SqliteCorrespondentRepository, SqliteDocumentRepository, SqliteDocumentTypeRepository,
    SqliteTagRepository,
};
pub use ingest_repo::{SqliteIngestLedger, SqliteSourceRepository};
pub use outbox::SqliteOutboxStore;
pub use search::SqliteSearchService;
pub use settings_repo::SqliteSettingsRepository;
pub use vault_repo::SqliteVaultRepository;
pub use workflow_repo::SqliteWorkflowRepository;

/// Create a new SQLite connection pool
pub async fn create_pool(database_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .foreign_keys(true);

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
    let fresh: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT 1 FROM sqlite_schema WHERE name = '_sqlx_migrations')",
    )
    .fetch_one(&mut *connection)
    .await?;
    sqlx::migrate!("../../migrations/sqlite")
        .run_direct(None, &mut *connection, false)
        .await?;
    if fresh {
        sqlx::query(super::CLEAR_PIPELINE_DEFAULTS)
            .execute(&mut *connection)
            .await?;
    }
    if file_backed {
        connection.close().await?;
    }
    Ok(())
}
