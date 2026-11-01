//! Domain models

mod correspondent;
mod document;
mod document_type;
mod settings;
mod tag;
mod webhook;
mod workflow;

pub use correspondent::{Correspondent, CreateCorrespondent, UpdateCorrespondent};
pub use document::{
    CreateDocument, Document, DocumentFilter, DocumentStatus, ProcessingStage, ProcessingStatus,
    UpdateDocument,
};
pub use document_type::{CreateDocumentType, DocumentType, UpdateDocumentType};
pub use settings::{
    AiSettings, EmbeddingSettings, OcrSettings, PipelineSettings, ProcessingSettings, Setting,
};
pub use tag::{CreateTag, Tag, UpdateTag};
pub use webhook::{NamedRef, WebhookDocument, WebhookPayload, WebhookUrls, PAYLOAD_VERSION};
pub use workflow::{
    glob_match, redact_url, validate_webhook_url, RunStatus, WebhookMethod, Workflow,
    WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun, WorkflowTrigger,
};

/// Unique identifier type alias
pub type Id = uuid::Uuid;
