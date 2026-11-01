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
pub mod postgres;
pub mod sqlite;

use std::sync::Arc;

use sqlx::{PgPool, SqlitePool};

use folio_core::ports::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, SearchService,
    SettingsRepository, TagRepository,
};

use crate::queue::JobQueue;

pub use health::DatabaseHealthCheck;
pub use postgres::{
    PostgresCorrespondentRepository, PostgresDocumentRepository, PostgresDocumentTypeRepository,
    PostgresSearchService, PostgresSettingsRepository, PostgresTagRepository,
};
pub use sqlite::{
    create_pool, run_migrations, SqliteCorrespondentRepository, SqliteDocumentRepository,
    SqliteDocumentTypeRepository, SqliteSearchService, SqliteSettingsRepository,
    SqliteTagRepository,
};

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
    pub documents: Arc<dyn DocumentRepository>,
    pub tags: Arc<dyn TagRepository>,
    pub correspondents: Arc<dyn CorrespondentRepository>,
    pub document_types: Arc<dyn DocumentTypeRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub search: Arc<dyn SearchService>,
    pub jobs: JobQueue,
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
            Self::Sqlite(pool) => Repositories {
                documents: Arc::new(SqliteDocumentRepository::new(pool.clone())),
                tags: Arc::new(SqliteTagRepository::new(pool.clone())),
                correspondents: Arc::new(SqliteCorrespondentRepository::new(pool.clone())),
                document_types: Arc::new(SqliteDocumentTypeRepository::new(pool.clone())),
                settings: Arc::new(SqliteSettingsRepository::new(pool.clone())),
                search: Arc::new(SqliteSearchService::new(pool.clone())),
                jobs: JobQueue::sqlite(pool.clone()),
            },
            Self::Postgres(pool) => Repositories {
                documents: Arc::new(PostgresDocumentRepository::new(pool.clone())),
                tags: Arc::new(PostgresTagRepository::new(pool.clone())),
                correspondents: Arc::new(PostgresCorrespondentRepository::new(pool.clone())),
                document_types: Arc::new(PostgresDocumentTypeRepository::new(pool.clone())),
                settings: Arc::new(PostgresSettingsRepository::new(pool.clone())),
                search: Arc::new(PostgresSearchService::new(pool.clone()).await),
                jobs: JobQueue::postgres(pool.clone()),
            },
        }
    }
}

/// Migrated throw-away database for tests; each pooled connection of an
/// in-memory SQLite database would see its own empty schema, so use a file.
#[cfg(test)]
pub(crate) async fn test_pool() -> (SqlitePool, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}?mode=rwc", dir.path().join("test.db").display());
    let pool = create_pool(&url).await.unwrap();
    run_migrations(&pool).await.unwrap();
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
