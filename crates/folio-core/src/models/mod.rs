//! Domain models

mod correspondent;
mod document;
mod document_type;
mod settings;
mod tag;

pub use correspondent::{Correspondent, CreateCorrespondent, UpdateCorrespondent};
pub use document::{CreateDocument, Document, DocumentFilter, DocumentStatus, UpdateDocument};
pub use document_type::{CreateDocumentType, DocumentType, UpdateDocumentType};
pub use settings::{AiSettings, ProcessingSettings, Setting};
pub use tag::{CreateTag, Tag, UpdateTag};

/// Unique identifier type alias
pub type Id = uuid::Uuid;
