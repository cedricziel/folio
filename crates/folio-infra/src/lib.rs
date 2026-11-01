//! Folio Infrastructure - External adapters
//!
//! This crate implements the port traits from folio-core:
//! - SQLite and PostgreSQL database repositories
//! - OCR and embedding providers behind a registry (see `docs/pipeline.md`)
//! - File storage
//! - Thumbnail generation
//! - Job queue for background processing
//! - Configuration loading

pub mod ai;
pub mod config;
pub mod database;
pub mod ingest;
pub mod queue;
pub mod storage;
pub mod thumbnail;
pub mod workflow;

pub use ai::{DynamicEmbeddingService, DynamicOcrService, PipelineContext};
pub use config::Config;
pub use database::{
    create_pool, run_migrations, Database, DatabaseBackend, DatabaseError, DatabaseHealthCheck,
    PostgresAuthStore, PostgresCorrespondentRepository, PostgresDocumentRepository,
    PostgresDocumentTypeRepository, PostgresIngestLedger, PostgresSearchService,
    PostgresSettingsRepository, PostgresSourceRepository, PostgresTagRepository,
    PostgresWorkflowRepository, Repositories, SqliteAuthStore, SqliteCorrespondentRepository,
    SqliteDocumentRepository, SqliteDocumentTypeRepository, SqliteIngestLedger,
    SqliteSearchService, SqliteSettingsRepository, SqliteSourceRepository, SqliteTagRepository,
    SqliteWorkflowRepository,
};
pub use ingest::{EmailSource, FolderSource, SourceFactory};
pub use queue::{Job, JobQueue, JobStatus, JobType, Worker, WorkerConfig};
pub use storage::FileStorage;
pub use thumbnail::ThumbnailService;
pub use workflow::{StorageFileMetadata, WebhookClient, WorkflowRunner};
