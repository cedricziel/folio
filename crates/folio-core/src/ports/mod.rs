//! Port traits (interfaces for external dependencies)
//!
//! These traits define the contracts that infrastructure adapters must implement.

mod access;
mod ai;
mod health;
mod ingest;
mod outbox;
mod repository;
mod saved_view;
mod storage;
mod workflow;

pub use access::AccessPolicy;
pub use ai::{
    EmbeddedChunk, EmbeddingBatch, EmbeddingService, OcrOutput, OcrService, SearchResult,
    SearchService,
};
pub use health::HealthCheck;
pub use ingest::{
    DocumentImporter, IngestLedger, IngestSink, IngestionSource, IngestionSourceFactory,
    SourceRepository, VaultIngest,
};
pub use outbox::OutboxStore;
pub use repository::{
    AuditLog, CorrespondentRepository, DocumentRepository, DocumentTypeRepository,
    MembershipRepository, SettingsRepository, TagRepository, VaultRepository,
};
pub use saved_view::{SavedViewAccess, SavedViewRepository};
pub use storage::{keys, purge_document, DocumentStore, LocalCopy, StoredObject};
pub use storage::{ProcessingQueue, ProcessingStatus};
pub use workflow::{FileMetadata, WorkflowRepository};
