//! PostgreSQL adapters

mod document_repo;
mod ingest_repo;
mod search;
mod settings_repo;
mod workflow_repo;

use std::str::FromStr;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

pub use document_repo::{
    PostgresCorrespondentRepository, PostgresDocumentRepository, PostgresDocumentTypeRepository,
    PostgresTagRepository,
};
pub use ingest_repo::{PostgresIngestLedger, PostgresSourceRepository};
pub use search::PostgresSearchService;
pub use settings_repo::PostgresSettingsRepository;
pub use workflow_repo::PostgresWorkflowRepository;

/// Create a new PostgreSQL connection pool
pub async fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let options = PgConnectOptions::from_str(database_url)?;

    PgPoolOptions::new()
        .max_connections(10)
        .connect_with(options)
        .await
}

/// Run PostgreSQL database migrations
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("../../migrations/postgres").run(pool).await
}

/// Map a sqlx error, turning unique violations into `Duplicate`.
pub(crate) fn map_write_error(
    e: sqlx::Error,
    entity_type: &'static str,
    value: &str,
) -> folio_core::error::Error {
    match e.as_database_error() {
        Some(db) if db.is_unique_violation() => folio_core::error::Error::Duplicate {
            entity_type,
            field: "name",
            value: value.to_string(),
        },
        _ => folio_core::error::Error::Repository(e.to_string()),
    }
}
