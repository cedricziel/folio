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

pub use ai::{AiServiceFactory, SqliteSearchService};
pub use config::Config;
pub use database::{
    create_pool, run_migrations, SqliteCorrespondentRepository, SqliteDocumentRepository,
    SqliteDocumentTypeRepository, SqliteSettingsRepository, SqliteTagRepository,
};
pub use queue::{Job, JobQueue, JobStatus, JobType, Worker, WorkerConfig};
pub use storage::FileStorage;
pub use thumbnail::ThumbnailService;
