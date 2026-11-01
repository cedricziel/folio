//! Folio Infrastructure - External adapters
//!
//! This crate implements the port traits from folio-core:
//! - SQLite database repositories
//! - OCR and embedding providers behind a registry (see `docs/pipeline.md`)
//! - File storage
//! - Thumbnail generation
//! - Job queue for background processing
//! - Configuration loading

pub mod ai;
pub mod config;
pub mod database;
pub mod queue;
pub mod storage;
pub mod thumbnail;

pub use ai::{DynamicEmbeddingService, DynamicOcrService, PipelineContext, SqliteSearchService};
pub use config::Config;
pub use database::{
    create_pool, run_migrations, SqliteCorrespondentRepository, SqliteDocumentRepository,
    SqliteDocumentTypeRepository, SqliteSettingsRepository, SqliteTagRepository,
};
pub use queue::{Job, JobQueue, JobStatus, JobType, Worker, WorkerConfig};
pub use storage::FileStorage;
pub use thumbnail::ThumbnailService;
