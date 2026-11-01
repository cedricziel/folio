//! Folio Infrastructure - External adapters
//!
//! This crate implements the port traits from folio-core:
//! - SQLite database repositories
//! - AI service providers (OpenAI, Anthropic, Local)
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
pub mod workflow;

pub use ai::{AiServiceFactory, DynamicEmbeddingService, DynamicOcrService, SqliteSearchService};
pub use config::Config;
pub use database::{
    create_pool, run_migrations, DatabaseHealthCheck, SqliteCorrespondentRepository,
    SqliteDocumentRepository, SqliteDocumentTypeRepository, SqliteSettingsRepository,
    SqliteTagRepository, SqliteWorkflowRepository,
};
pub use queue::{Job, JobQueue, JobStatus, JobType, Worker, WorkerConfig};
pub use storage::FileStorage;
pub use thumbnail::ThumbnailService;
pub use workflow::{StorageFileMetadata, WebhookClient, WorkflowRunner};
