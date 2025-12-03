//! Folio Infrastructure - External adapters
//!
//! This crate implements the port traits from folio-core:
//! - SQLite database repositories
//! - AI service providers (OpenAI, Anthropic, Local)
//! - File storage
//! - Configuration loading

pub mod ai;
pub mod config;
pub mod database;
pub mod storage;

pub use ai::{AiServiceFactory, SqliteSearchService};
pub use config::Config;
pub use database::{
    create_pool, run_migrations, SqliteCorrespondentRepository, SqliteDocumentRepository,
    SqliteDocumentTypeRepository, SqliteTagRepository,
};
pub use storage::FileStorage;
