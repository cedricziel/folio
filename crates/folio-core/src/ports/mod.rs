//! Port traits (interfaces for external dependencies)
//!
//! These traits define the contracts that infrastructure adapters must implement.

mod ai;
mod repository;

pub use ai::{EmbeddingService, OcrService, SearchResult, SearchService};
pub use repository::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, TagRepository,
};
