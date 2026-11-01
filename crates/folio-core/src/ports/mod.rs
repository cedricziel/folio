//! Port traits (interfaces for external dependencies)
//!
//! These traits define the contracts that infrastructure adapters must implement.

mod ai;
mod health;
mod ingest;
mod repository;
mod workflow;

pub use ai::{EmbeddingService, OcrService, SearchResult, SearchService};
pub use health::HealthCheck;
pub use ingest::{
    DocumentImporter, ImportRequest, IngestLedger, IngestSink, IngestionSource,
    IngestionSourceFactory, SourceRepository,
};
pub use repository::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, SettingsRepository,
    TagRepository,
};
pub use workflow::{FileMetadata, WorkflowJobQueue, WorkflowRepository};
