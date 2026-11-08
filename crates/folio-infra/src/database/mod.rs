//! Database adapters
//!
//! The backend follows the scheme of `database.url`: `sqlite:` or
//! `postgres://` (`postgresql://`). Each backend has its own repository
//! implementations behind the same ports; [`Database::repositories`] hands
//! them out as trait objects.
//!
//! Queries are plain runtime `sqlx::query_as` calls with `FromRow` structs
//! rather than `query!` macros, so building needs neither a live database nor
//! an offline cache per backend.

mod health;
mod ingest_outcomes;
mod metadata_value;
pub mod postgres;
mod saved_view;
mod source_lifecycle;
pub mod sqlite;
mod workflow_lifecycle;

use std::sync::Arc;

use sqlx::{PgPool, SqlitePool};
use tokio::sync::Notify;

use folio_auth::AuthStore;
use folio_core::ports::{
    AccessPolicy, AuditLog, CorrespondentRepository, DocumentRepository, DocumentTypeRepository,
    IngestLedger, IngestOutcomeRepository, MembershipRepository, OutboxStore, SearchService,
    SettingsRepository, SourceRepository, TagRepository, VaultRepository, WorkflowRepository,
};
use folio_core::EventOrigin;

use crate::queue::JobQueue;

pub use health::DatabaseHealthCheck;
pub use postgres::{
    PostgresAuthStore, PostgresCorrespondentRepository, PostgresDocumentRepository,
    PostgresDocumentTypeRepository, PostgresIngestLedger, PostgresOutboxStore,
    PostgresSearchService, PostgresSettingsRepository, PostgresSourceRepository,
    PostgresTagRepository, PostgresVaultRepository, PostgresWorkflowRepository,
};
pub use sqlite::{
    create_pool, run_migrations, SqliteAuthStore, SqliteCorrespondentRepository,
    SqliteDocumentRepository, SqliteDocumentTypeRepository, SqliteIngestLedger, SqliteOutboxStore,
    SqliteSearchService, SqliteSettingsRepository, SqliteSourceRepository, SqliteTagRepository,
    SqliteVaultRepository, SqliteWorkflowRepository,
};

/// The error for an import whose insert hit a unique index: `Conflict` when
/// the document already exists, otherwise the database error
async fn import_collision(
    documents: &dyn DocumentRepository,
    vault: folio_core::models::VaultId,
    document: &folio_core::models::ImportedDocument,
    error: sqlx::Error,
) -> folio_core::Error {
    let existing = documents
        .find_by_external_ref(vault, &document.external_ref)
        .await;
    let id_taken = documents.locate_retained(document.id).await;
    match (existing, id_taken) {
        (Ok(Some(_)), _) | (_, Ok(Some(_))) => {
            folio_core::Error::Conflict("Document was already imported".into())
        }
        _ => folio_core::Error::Repository(error.to_string()),
    }
}

const CLEAR_PIPELINE_DEFAULTS: &str = "DELETE FROM settings WHERE key LIKE 'ocr.%' \
    OR key LIKE 'embeddings.%' OR key LIKE 'processing.%'";

/// Supported database backends
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseBackend {
    Sqlite,
    Postgres,
}

impl DatabaseBackend {
    /// Pick the backend from a database URL scheme.
    pub fn from_url(url: &str) -> Result<Self, DatabaseError> {
        if url.starts_with("sqlite:") {
            Ok(Self::Sqlite)
        } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Ok(Self::Postgres)
        } else {
            Err(DatabaseError::UnsupportedUrl(url.to_string()))
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("unsupported database URL scheme (expected sqlite: or postgres://): {0}")]
    UnsupportedUrl(String),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// A connected database of either backend
#[derive(Clone)]
pub enum Database {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl From<SqlitePool> for Database {
    fn from(pool: SqlitePool) -> Self {
        Self::Sqlite(pool)
    }
}

impl From<PgPool> for Database {
    fn from(pool: PgPool) -> Self {
        Self::Postgres(pool)
    }
}

/// The ports Folio needs, implemented for one backend
pub struct Repositories {
    pub entries: Arc<dyn folio_core::ports::DocumentEntryRepository>,
    pub upload_receipts: Arc<dyn folio_core::ports::UploadReceiptRepository>,
    pub saved_views: Arc<dyn folio_core::ports::SavedViewRepository>,
    /// Writes an outbox event with every lifecycle change
    pub documents: Arc<dyn DocumentRepository>,
    /// Like `documents`, but its events are attributed to workflows, so they
    /// reach live clients without triggering workflows again
    pub workflow_documents: Arc<dyn DocumentRepository>,
    pub outbox: Arc<dyn OutboxStore>,
    pub tags: Arc<dyn TagRepository>,
    pub correspondents: Arc<dyn CorrespondentRepository>,
    pub document_types: Arc<dyn DocumentTypeRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub workflows: Arc<dyn WorkflowRepository>,
    pub sources: Arc<dyn SourceRepository>,
    pub ledger: Arc<dyn IngestLedger>,
    pub ingest_outcomes: Arc<dyn IngestOutcomeRepository>,
    pub import_runs: Arc<dyn folio_core::ports::ImportRunRepository>,
    pub import_items: Arc<dyn folio_core::ports::ImportItemRepository>,
    pub paperless_ledger: Arc<dyn folio_core::ports::PaperlessLedgerRepository>,
    pub search: Arc<dyn SearchService>,
    pub jobs: JobQueue,
    pub auth: Arc<dyn AuthStore>,
    pub vaults: Arc<dyn VaultRepository>,
    pub members: Arc<dyn MembershipRepository>,
    pub audit: Arc<dyn AuditLog>,
    pub access: Arc<dyn AccessPolicy>,
    pub push: Arc<dyn folio_core::ports::PushDeviceRepository>,
    pub grants: Arc<dyn folio_core::ports::GrantLiveness>,
}

impl Database {
    /// Connect to the database named by `url`.
    pub async fn connect(url: &str) -> Result<Self, DatabaseError> {
        match DatabaseBackend::from_url(url)? {
            DatabaseBackend::Sqlite => Ok(Self::Sqlite(sqlite::create_pool(url).await?)),
            DatabaseBackend::Postgres => Ok(Self::Postgres(postgres::create_pool(url).await?)),
        }
    }

    pub fn backend(&self) -> DatabaseBackend {
        match self {
            Self::Sqlite(_) => DatabaseBackend::Sqlite,
            Self::Postgres(_) => DatabaseBackend::Postgres,
        }
    }

    /// Apply the migrations for this backend.
    pub async fn migrate(&self) -> Result<(), DatabaseError> {
        match self {
            Self::Sqlite(pool) => sqlite::run_migrations(pool).await?,
            Self::Postgres(pool) => postgres::run_migrations(pool).await?,
        }
        Ok(())
    }

    /// Build the repositories. Call after [`Database::migrate`]: the
    /// PostgreSQL search service probes for pgvector and adjusts the schema.
    pub async fn repositories(&self) -> Repositories {
        match self {
            Self::Sqlite(pool) => {
                let vaults = Arc::new(SqliteVaultRepository::new(pool.clone()));
                let push = Arc::new(sqlite::SqlitePushRepository::new(pool.clone()));
                let imports = Arc::new(sqlite::SqliteImportRepository::new(pool.clone()));
                let wake = Arc::new(Notify::new());
                let documents = |origin| {
                    Arc::new(
                        SqliteDocumentRepository::new(pool.clone())
                            .with_origin(origin)
                            .with_wake(wake.clone()),
                    )
                };
                Repositories {
                    entries: Arc::new(
                        sqlite::SqliteDocumentEntryRepository::new(pool.clone())
                            .with_wake(wake.clone()),
                    ),
                    upload_receipts: Arc::new(sqlite::SqliteUploadReceiptRepository::new(
                        pool.clone(),
                    )),
                    saved_views: Arc::new(
                        sqlite::SqliteSavedViewRepository::new(pool.clone())
                            .with_wake(wake.clone()),
                    ),
                    documents: documents(EventOrigin::User),
                    workflow_documents: documents(EventOrigin::Workflow),
                    outbox: Arc::new(SqliteOutboxStore::new(pool.clone()).with_wake(wake.clone())),
                    tags: Arc::new(SqliteTagRepository::new(pool.clone())),
                    correspondents: Arc::new(SqliteCorrespondentRepository::new(pool.clone())),
                    document_types: Arc::new(SqliteDocumentTypeRepository::new(pool.clone())),
                    settings: Arc::new(SqliteSettingsRepository::new(pool.clone())),
                    workflows: Arc::new(SqliteWorkflowRepository::new(pool.clone())),
                    sources: Arc::new(SqliteSourceRepository::new(pool.clone())),
                    ledger: Arc::new(SqliteIngestLedger::new(pool.clone())),
                    ingest_outcomes: Arc::new(sqlite::SqliteIngestOutcomeRepository::new(
                        pool.clone(),
                    )),
                    import_runs: imports.clone(),
                    import_items: imports.clone(),
                    paperless_ledger: imports,
                    search: Arc::new(SqliteSearchService::new(pool.clone())),
                    jobs: JobQueue::sqlite(pool.clone()),
                    auth: Arc::new(SqliteAuthStore::new(pool.clone())),
                    vaults: vaults.clone(),
                    members: vaults.clone(),
                    audit: vaults.clone(),
                    access: vaults,
                    push: push.clone(),
                    grants: push,
                }
            }
            Self::Postgres(pool) => {
                let vaults = Arc::new(PostgresVaultRepository::new(pool.clone()));
                let push = Arc::new(postgres::PostgresPushRepository::new(pool.clone()));
                let imports = Arc::new(postgres::PostgresImportRepository::new(pool.clone()));
                let wake = Arc::new(Notify::new());
                let documents = |origin| {
                    Arc::new(
                        PostgresDocumentRepository::new(pool.clone())
                            .with_origin(origin)
                            .with_wake(wake.clone()),
                    )
                };
                Repositories {
                    entries: Arc::new(
                        postgres::PostgresDocumentEntryRepository::new(pool.clone())
                            .with_wake(wake.clone()),
                    ),
                    upload_receipts: Arc::new(postgres::PostgresUploadReceiptRepository::new(
                        pool.clone(),
                    )),
                    saved_views: Arc::new(
                        postgres::PostgresSavedViewRepository::new(pool.clone())
                            .with_wake(wake.clone()),
                    ),
                    documents: documents(EventOrigin::User),
                    workflow_documents: documents(EventOrigin::Workflow),
                    outbox: Arc::new(
                        PostgresOutboxStore::new(pool.clone()).with_wake(wake.clone()),
                    ),
                    tags: Arc::new(PostgresTagRepository::new(pool.clone())),
                    correspondents: Arc::new(PostgresCorrespondentRepository::new(pool.clone())),
                    document_types: Arc::new(PostgresDocumentTypeRepository::new(pool.clone())),
                    settings: Arc::new(PostgresSettingsRepository::new(pool.clone())),
                    workflows: Arc::new(PostgresWorkflowRepository::new(pool.clone())),
                    sources: Arc::new(PostgresSourceRepository::new(pool.clone())),
                    ledger: Arc::new(PostgresIngestLedger::new(pool.clone())),
                    ingest_outcomes: Arc::new(postgres::PostgresIngestOutcomeRepository::new(
                        pool.clone(),
                    )),
                    import_runs: imports.clone(),
                    import_items: imports.clone(),
                    paperless_ledger: imports,
                    search: Arc::new(PostgresSearchService::new(pool.clone()).await),
                    jobs: JobQueue::postgres(pool.clone()),
                    auth: Arc::new(PostgresAuthStore::new(pool.clone())),
                    vaults: vaults.clone(),
                    members: vaults.clone(),
                    audit: vaults.clone(),
                    access: vaults,
                    push: push.clone(),
                    grants: push,
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn test_vault() -> folio_core::models::VaultId {
    folio_core::models::VaultId(uuid::Uuid::nil())
}

/// Migrated throw-away database for tests; each pooled connection of an
/// in-memory SQLite database would see its own empty schema, so use a file.
#[cfg(test)]
pub(crate) async fn test_pool() -> (SqlitePool, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("test.db").display());
    let pool = create_pool(&url).await.unwrap();
    run_migrations(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO vaults (id, name, created_at, updated_at) VALUES ($1, 'Test', $2, $2)",
    )
    .bind(test_vault().to_string())
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    .unwrap();
    (pool, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_follows_url_scheme() {
        let cases = [
            ("sqlite:folio.db?mode=rwc", Some(DatabaseBackend::Sqlite)),
            ("sqlite::memory:", Some(DatabaseBackend::Sqlite)),
            ("postgres://u:p@h/db", Some(DatabaseBackend::Postgres)),
            ("postgresql://u:p@h/db", Some(DatabaseBackend::Postgres)),
            ("mysql://h/db", None),
            ("folio.db", None),
        ];
        for (url, expected) in cases {
            assert_eq!(DatabaseBackend::from_url(url).ok(), expected, "{url}");
        }
    }
}
