//! Domain models

mod correspondent;
mod document;
mod document_date;
mod document_page;
mod document_review;
mod document_type;
mod file_identity;
mod ingest;
mod metadata;
mod settings;
mod source;
mod tag;
mod vault;
mod webhook;
mod workflow;

pub use correspondent::{Correspondent, CreateCorrespondent, UpdateCorrespondent};
pub use document::{
    CreateDocument, Document, DocumentFilter, DocumentStatus, ProcessingStage, ProcessingStatus,
    UpdateDocument,
};
pub use document_date::DocumentDate;
pub use document_page::{DocumentCursor, DocumentPage, DocumentPageQuery, DocumentSort};
pub use document_review::{DocumentReview, ReviewDecision, ReviewOrigin};
pub use document_type::{CreateDocumentType, DocumentType, UpdateDocumentType};
pub use file_identity::FileIdentity;
pub use ingest::{
    EmailConfig, FolderAfterImport, FolderConfig, IngestItem, IngestOutcome, IngestionSourceDef,
    IngestionSourceInput, LedgerEntry, MailAfterImport, MailBody, MailSecurity, PollReport,
    SourceConfig, SourceOrigin, SourceStatus, REDACTED,
};
pub use metadata::{MetadataKind, MetadataValue};
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
pub use webhook::{NamedRef, WebhookDocument, WebhookPayload, WebhookUrls, PAYLOAD_VERSION};
pub use workflow::{
    glob_match, redact_url, validate_webhook_url, RunStatus, WebhookMethod, Workflow,
    WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun, WorkflowTrigger,
};

/// Unique identifier type alias
pub type Id = uuid::Uuid;
