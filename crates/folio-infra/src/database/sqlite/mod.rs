//! SQLite adapters

mod auth_repo;
mod document_repo;
mod ingest_repo;
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
    sqlx::migrate!("../../migrations/sqlite").run(pool).await
}
