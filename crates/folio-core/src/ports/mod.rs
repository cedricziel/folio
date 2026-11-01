//! Port traits (interfaces for external dependencies)
//!
//! These traits define the contracts that infrastructure adapters must implement.

mod ai;
mod repository;
mod storage;

pub use ai::{EmbeddingService, OcrService, SearchResult, SearchService};
pub use repository::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, SettingsRepository,
    TagRepository,
};
pub use storage::{FileStore, ProcessingQueue, ProcessingStatus};
