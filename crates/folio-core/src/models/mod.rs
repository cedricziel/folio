//! Domain models

mod correspondent;
mod document;
mod document_type;
mod ingest;
mod settings;
mod source;
mod tag;
mod webhook;
mod workflow;

pub use correspondent::{Correspondent, CreateCorrespondent, UpdateCorrespondent};
pub use document::{CreateDocument, Document, DocumentFilter, UpdateDocument};
pub use document_type::{CreateDocumentType, DocumentType, UpdateDocumentType};
pub use ingest::{
    EmailConfig, FolderAfterImport, FolderConfig, IngestItem, IngestOutcome, IngestionSourceDef,
    IngestionSourceInput, LedgerEntry, MailAfterImport, MailBody, MailSecurity, PollReport,
    SourceConfig, SourceOrigin, SourceStatus, REDACTED,
};
pub use settings::{AiSettings, ProcessingSettings, Setting};
pub use source::{
    DocumentSource, EmailOrigin, FolderOrigin, SourceConditions, SourceKind, TextMatch,
};
pub use tag::{CreateTag, Tag, UpdateTag};
pub use webhook::{NamedRef, WebhookDocument, WebhookPayload, WebhookUrls, PAYLOAD_VERSION};
pub use workflow::{
    glob_match, redact_url, validate_webhook_url, RunStatus, WebhookMethod, Workflow,
    WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun, WorkflowTrigger,
};

/// Unique identifier type alias
pub type Id = uuid::Uuid;
