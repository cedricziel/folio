//! Port traits (interfaces for external dependencies)
//!
//! These traits define the contracts that infrastructure adapters must implement.

mod access;
mod ai;
mod caller;
mod document_entries;
mod health;
mod import;
mod import_runs;
mod ingest;
mod metadata;
mod outbox;
mod repository;
mod saved_view;
mod storage;
mod upload_receipts;
mod workflow;

pub use access::AccessPolicy;
pub use ai::{
    EmbeddedChunk, EmbeddingBatch, EmbeddingService, OcrOutput, OcrService, SearchResult,
    SearchService,
};
pub use caller::CallerResolver;
pub use document_entries::{DocumentEntryRepository, EntryCreation};
pub use health::HealthCheck;
pub use import::{
    digest_original, ChecksumAlgorithm, CompleteIdSet, FileDigests, InvalidDocumentReason,
    MalformedKind, OriginalFile, PaperlessChecksum, PaperlessCustomField,
    PaperlessCustomFieldInstance, PaperlessDocument, PaperlessFieldType, PaperlessFieldValue,
    PaperlessInvalidDocument, PaperlessNamed, PaperlessNote, PaperlessReadError, PaperlessReader,
    PaperlessRecord, PaperlessSelectOption, PaperlessSourceInfo, PaperlessTag, PaperlessVersion,
    PaperlessVersionInfo, RecordStream, SourceFailure, SourceLocation,
};
pub use import_runs::{ImportItemRepository, ImportRunRepository, PaperlessLedgerRepository};
pub use ingest::{
    DocumentImporter, IngestLedger, IngestOutcomeReader, IngestOutcomeRepository, IngestSink,
    IngestionSource, IngestionSourceFactory, SourceRepository, VaultIngest,
};
pub use metadata::{MetadataFieldRepository, MetadataOptionRepository, MetadataValueRepository};
pub use outbox::OutboxStore;
pub use repository::{
    AuditLog, CorrespondentRepository, DocumentRepository, DocumentTypeRepository,
    MembershipRepository, SettingsRepository, TagRepository, VaultRepository,
};
pub use saved_view::{SavedViewAccess, SavedViewRepository};
pub use storage::{keys, purge_document, DocumentStore, LocalCopy, StoredObject};
pub use storage::{ProcessingQueue, ProcessingStatus};
pub use upload_receipts::{
    upload_fingerprint, ExpiredReservation, UploadClaim, UploadReceiptKey, UploadReceiptRepository,
    UPLOAD_CLAIM_HEARTBEAT, UPLOAD_CLAIM_TIMEOUT, UPLOAD_RECEIPT_RETENTION,
};
pub use workflow::{FileMetadata, WorkflowRepository};
