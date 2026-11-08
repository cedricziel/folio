//! Domain models

mod correspondent;
mod document;
mod document_date;
mod document_entry;
mod document_page;
mod document_review;
mod document_type;
mod file_identity;
mod import_run;
mod imported_document;
mod ingest;
mod ingest_identity;
mod ingest_outcome;
mod metadata;
mod metadata_field;
mod metadata_value;
mod processing_attempt;
mod saved_view;
mod settings;
mod source;
mod tag;
mod vault;
mod webhook;
mod workflow;
mod workflow_event_conditions;
mod workflow_trigger;

pub use correspondent::{Correspondent, CreateCorrespondent, UpdateCorrespondent};
pub use document::{
    CreateDocument, Document, DocumentFilter, DocumentStatus, ProcessingStage, ProcessingStatus,
    UpdateDocument,
};
pub use document_date::DocumentDate;
pub use document_entry::{
    DocumentEntry, EntryBody, EntryCursor, EntryKind, EntryPage, EntryPageQuery, EntryPermissions,
};
pub use document_page::{DocumentCursor, DocumentPage, DocumentPageQuery, DocumentSort};
pub use document_review::{DocumentReview, ReviewDecision, ReviewOrigin};
pub use document_type::{CreateDocumentType, DocumentType, UpdateDocumentType};
pub use file_identity::FileIdentity;
pub use import_run::{
    CreateImportRun, FolioState, ImportAction, ImportApplied, ImportItem, ImportItemCount,
    ImportItemOutcome, ImportItemStatus, ImportRun, ImportRunKind, ImportRunStatus, ImportTrigger,
    LibrarySample, NewImportItem, NewImportRun, PaperlessLedgerEntry, PaperlessSyncState,
    RunTransition, TaxonomyKind, TaxonomyMapping, IMPORT_CLAIM_TIMEOUT,
};
pub use imported_document::{ExternalRef, ImportSource, ImportedDocument, ImportedNote};
pub use ingest::{
    EmailConfig, FolderAfterImport, FolderConfig, IngestItem, IngestOutcome, IngestionSourceDef,
    IngestionSourceInput, LedgerEntry, MailAfterImport, MailBody, MailSecurity, PollReport,
    SourceConfig, SourceLifecycle, SourceOrigin, SourceStatus, REDACTED,
};
pub use ingest_identity::IngestItemKey;
pub use ingest_outcome::{
    IngestAttempt, IngestClaim, IngestOutcomePage, IngestOutcomeRecord, IngestResult,
    SourceHealthChange,
};
pub use metadata::{MetadataKind, MetadataValue};
pub use metadata_field::{MetadataField, MetadataFieldInput, MetadataOption, MetadataOptionInput};
pub use metadata_value::{DocumentMetadata, DocumentMetadataValue, MetadataChange};
pub use processing_attempt::{AttemptOutcome, ProcessingAttempt, StageOutcome};
pub use saved_view::{SavedView, SavedViewInput, SavedViewQuery, ViewPresentation, ViewVisibility};
pub use settings::{
    AiSettings, EmbeddingSettings, OcrSettings, PipelineSettings, ProcessingSettings, Setting,
};
pub use source::{
    DocumentSource, EmailOrigin, FolderOrigin, SourceConditions, SourceKind, TextMatch,
};
pub use tag::{CreateTag, Tag, UpdateTag};
pub use vault::{
    Access, AuditEvent, GroupId, NewAuditEvent, Principal, UserId, Vault, VaultAction, VaultId,
    VaultMember, VaultRole,
};
pub use webhook::{
    NamedRef, WebhookDocument, WebhookPayload, WebhookUrls, WorkflowEventIdentity, PAYLOAD_VERSION,
};
pub use workflow::{
    glob_match, redact_url, validate_webhook_url, RunStatus, WebhookMethod, Workflow,
    WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun, WorkflowTrigger,
};

pub use workflow_event_conditions::{WorkflowActorCondition, WorkflowEventConditions};
pub use workflow_trigger::{
    WorkflowConditionField, WorkflowConditionType, WorkflowTarget, WorkflowTriggerDefinition,
    WorkflowTriggerRegistry,
};

/// Unique identifier type alias
pub type Id = uuid::Uuid;
