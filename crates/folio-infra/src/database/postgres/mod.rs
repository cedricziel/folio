//! PostgreSQL adapters

mod metadata_repo;
pub use metadata_repo::PostgresMetadataFieldRepository;

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

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{migrate::Migrate, Connection, PgPool};

pub use auth_repo::PostgresAuthStore;
pub use document_entries::PostgresDocumentEntryRepository;
pub use document_repo::{
    PostgresCorrespondentRepository, PostgresDocumentRepository, PostgresDocumentTypeRepository,
    PostgresTagRepository,
};
pub use ingest_outcomes::PostgresIngestOutcomeRepository;
pub use ingest_repo::{PostgresIngestLedger, PostgresSourceRepository};
pub use outbox::{
    insert_domain_event, insert_domain_event_with_origin, insert_event_with_envelope,
    PostgresOutboxStore,
};
pub use search::PostgresSearchService;
pub use settings_repo::PostgresSettingsRepository;
pub use vault_repo::PostgresVaultRepository;
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
    let mut connection = pool.acquire().await?;
    connection.close_on_drop();
    connection.lock().await?;
    sqlx::migrate!("../../migrations/postgres")
        .set_locking(false)
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
    connection.close().await?;
    Ok(())
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

mod saved_view_repo;
pub use saved_view_repo::PostgresSavedViewRepository;
