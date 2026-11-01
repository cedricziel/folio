//! Database adapters (SQLite)

mod document_repo;
mod health;
mod ingest_repo;
mod settings_repo;
mod workflow_repo;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

pub use document_repo::{
    SqliteCorrespondentRepository, SqliteDocumentRepository, SqliteDocumentTypeRepository,
    SqliteTagRepository,
};
pub use health::DatabaseHealthCheck;
pub use ingest_repo::{SqliteIngestLedger, SqliteSourceRepository};
pub use settings_repo::SqliteSettingsRepository;
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

/// Run database migrations
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("../../migrations").run(pool).await
}
