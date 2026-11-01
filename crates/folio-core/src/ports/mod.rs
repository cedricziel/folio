//! Port traits (interfaces for external dependencies)
//!
//! These traits define the contracts that infrastructure adapters must implement.

mod ai;
mod health;
mod repository;
mod storage;
mod workflow;

pub use ai::{
    EmbeddedChunk, EmbeddingBatch, EmbeddingService, OcrOutput, OcrService, SearchResult,
    SearchService,
};
pub use health::HealthCheck;
pub use repository::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, SettingsRepository,
    TagRepository,
};
pub use storage::{keys, purge_document, DocumentStore, LocalCopy, StoredObject};
pub use storage::{ProcessingQueue, ProcessingStatus};
pub use workflow::{FileMetadata, WorkflowJobQueue, WorkflowRepository};
