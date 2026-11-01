//! Folio Infrastructure - External adapters
//!
//! This crate implements the port traits from folio-core:
//! - SQLite and PostgreSQL database repositories
//! - AI service providers (OpenAI, Anthropic, Local)
//! - Document storage (local filesystem, S3)
//! - Thumbnail generation
//! - Job queue for background processing
//! - Configuration loading

pub mod ai;
pub mod config;
pub mod database;
pub mod queue;
pub mod storage;
pub mod telemetry;
pub mod thumbnail;

pub use ai::{AiServiceFactory, DynamicEmbeddingService, DynamicOcrService};
pub use config::Config;
pub use database::{
    create_pool, run_migrations, Database, DatabaseBackend, DatabaseError, DatabaseHealthCheck,
    PostgresAuthStore, PostgresCorrespondentRepository, PostgresDocumentRepository,
    PostgresDocumentTypeRepository, PostgresSearchService, PostgresSettingsRepository,
    PostgresTagRepository, Repositories, SqliteAuthStore, SqliteCorrespondentRepository,
    SqliteDocumentRepository, SqliteDocumentTypeRepository, SqliteSearchService,
    SqliteSettingsRepository, SqliteTagRepository,
};
pub use queue::{Job, JobQueue, JobStatus, JobType, Worker, WorkerConfig};
pub use storage::{
    build_store, cleanup_legacy_layout, migrate_legacy_layout, ObjectStoreDocumentStore,
};
pub use thumbnail::ThumbnailService;
