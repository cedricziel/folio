//! Folio Infrastructure - External adapters
//!
//! This crate implements the port traits from folio-core:
//! - SQLite and PostgreSQL database repositories
//! - OCR and embedding providers behind a registry (see `docs/pipeline.md`)
//! - Document storage (local filesystem, S3)
//! - Thumbnail generation
//! - Job queue for background processing
//! - Configuration loading
//! - Import sources and their path checks

pub mod ai;
pub mod caller;
pub mod config;
pub mod database;
pub mod import;
pub mod ingest;
pub mod queue;
pub mod storage;
pub mod telemetry;
pub mod thumbnail;
pub mod workflow;

pub use ai::{DynamicEmbeddingService, DynamicOcrService, PipelineContext};
pub use caller::AuthCallerResolver;
pub use config::Config;
pub use database::{
    create_pool, run_migrations, Database, DatabaseBackend, DatabaseError, DatabaseHealthCheck,
    PostgresAuthStore, PostgresCorrespondentRepository, PostgresDocumentRepository,
    PostgresDocumentTypeRepository, PostgresIngestLedger, PostgresOutboxStore,
    PostgresSearchService, PostgresSettingsRepository, PostgresSourceRepository,
    PostgresTagRepository, PostgresWorkflowRepository, Repositories, SqliteAuthStore,
    SqliteCorrespondentRepository, SqliteDocumentRepository, SqliteDocumentTypeRepository,
    SqliteIngestLedger, SqliteOutboxStore, SqliteSearchService, SqliteSettingsRepository,
    SqliteSourceRepository, SqliteTagRepository, SqliteWorkflowRepository,
};
pub use ingest::{EmailSource, FolderSource, SourceFactory};
pub use queue::{FailOutcome, Job, JobClass, JobQueue, JobStatus, JobType, Worker, WorkerConfig};
pub use storage::{build_store, ObjectStoreDocumentStore};
pub use thumbnail::ThumbnailService;
pub use workflow::{StorageFileMetadata, WebhookClient, WorkflowRunner};
