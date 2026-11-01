//! Port traits (interfaces for external dependencies)
//!
//! These traits define the contracts that infrastructure adapters must implement.

mod ai;
mod repository;

pub use ai::{
    EmbeddedChunk, EmbeddingBatch, EmbeddingService, OcrOutput, OcrService, SearchResult,
    SearchService,
};
pub use repository::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, SettingsRepository,
    TagRepository,
};
