//! Document model

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{DocumentSource, Id, UserId, VaultId};

/// A stage of the document processing pipeline that is tracked per document
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingStage {
    Ocr,
    Embed,
}

/// State of one pipeline stage for one document
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ProcessingStatus {
    /// Waiting to be processed
    #[default]
    Pending,
    /// A worker is processing it right now
    Running,
    /// Finished successfully (possibly with an empty result)
    Done,
    /// The last attempt failed
    Failed,
    /// Deliberately not run (disabled in settings, or nothing to process)
    Skipped,
}

impl ProcessingStatus {
    /// Whether the stage still has work ahead of it
    pub fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

impl std::str::FromStr for ProcessingStatus {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            other => Err(format!("unknown processing status: {other}")),
        }
    }
}

impl std::fmt::Display for ProcessingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A document in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Document {
    /// Unique identifier
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,

    /// The vault the document belongs to
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,

    /// Who uploaded the document; `None` for documents that predate vaults
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub uploaded_by: Option<UserId>,

    /// Document title
    pub title: String,

    /// Extracted text content (from OCR or text extraction)
    pub content: Option<String>,

    /// Original filename when uploaded
    pub original_filename: Option<String>,

    /// Path to the stored file (relative to storage root)
    pub file_path: Option<String>,

    /// MIME type of the original document
    pub mime_type: Option<String>,

    /// Associated correspondent (sender)
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub correspondent_id: Option<Id>,

    /// Document type classification
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub document_type_id: Option<Id>,

    /// Number of pages in the document (for PDFs and multi-page images)
    pub page_count: Option<u32>,

    /// How the document entered Folio
    #[serde(default)]
    pub source: DocumentSource,

    /// State of the OCR stage
    pub ocr_status: ProcessingStatus,

    /// State of the embedding stage
    pub embed_status: ProcessingStatus,

    #[serde(default)]
    pub deleted_at: Option<DateTime<Utc>>,

    #[serde(default)]
    pub purge_requested_at: Option<DateTime<Utc>>,

    /// When the document was created in the system
    pub created_at: DateTime<Utc>,

    /// When the document was last updated
    pub updated_at: DateTime<Utc>,
}

impl Document {
    /// Create a new document with the given title in a vault
    pub fn new(vault_id: VaultId, title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Id::new_v4(),
            vault_id,
            uploaded_by: None,
            title: title.into(),
            content: None,
            original_filename: None,
            file_path: None,
            mime_type: None,
            correspondent_id: None,
            document_type_id: None,
            page_count: None,
            source: DocumentSource::default(),
            ocr_status: ProcessingStatus::Pending,
            embed_status: ProcessingStatus::Pending,
            deleted_at: None,
            purge_requested_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set the document content
    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self.ocr_status = ProcessingStatus::Done;
        self
    }

    /// Set the original filename
    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.original_filename = Some(filename.into());
        self
    }

    /// Set the file path
    pub fn with_file_path(mut self, path: impl Into<String>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    /// Set the MIME type
    pub fn with_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    /// Set the correspondent
    pub fn with_correspondent(mut self, correspondent_id: Id) -> Self {
        self.correspondent_id = Some(correspondent_id);
        self
    }

    /// Set the document type
    pub fn with_document_type(mut self, document_type_id: Id) -> Self {
        self.document_type_id = Some(document_type_id);
        self
    }
}

/// Where a document stands between upload and being filed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    /// Text has not been extracted yet
    Processing,
    /// Folio could not read the file
    Failed,
    /// Read, but not yet assigned a correspondent or a document type
    NeedsReview,
    /// Read and assigned a correspondent or a document type
    Filed,
}

impl Document {
    /// Derive the document's inbox status from what is known about it
    pub fn status(&self) -> DocumentStatus {
        if self.ocr_status.is_active() {
            DocumentStatus::Processing
        } else if self.ocr_status == ProcessingStatus::Failed {
            DocumentStatus::Failed
        } else if self.correspondent_id.is_none() && self.document_type_id.is_none() {
            DocumentStatus::NeedsReview
        } else {
            DocumentStatus::Filed
        }
    }
}

/// Parameters for creating a new document
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateDocument {
    /// Recorded so contributors can later edit their own uploads
    pub uploaded_by: Option<UserId>,
    pub title: String,
    pub content: Option<String>,
    pub original_filename: Option<String>,
    pub mime_type: Option<String>,
    pub correspondent_id: Option<Id>,
    pub document_type_id: Option<Id>,
    pub source: DocumentSource,
    /// Tags attached together with the record, so `document.uploaded` already sees them
    pub tag_ids: Vec<Id>,
}

/// Parameters for updating a document
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateDocument {
    pub title: Option<String>,
    pub content: Option<String>,
    pub correspondent_id: Option<Option<Id>>,
    pub document_type_id: Option<Option<Id>>,
    pub tag_ids: Option<Vec<Id>>,
}

/// Filter criteria for listing documents
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocumentFilter {
    pub correspondent_id: Option<Id>,
    pub document_type_id: Option<Id>,
    pub tag_ids: Option<Vec<Id>>,
    pub search_query: Option<String>,
    /// Only documents created at or after this instant
    pub created_after: Option<DateTime<Utc>>,
    /// Only documents created strictly before this instant
    pub created_before: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_follows_extraction_and_classification() {
        let doc = Document::new(VaultId::new(), "Invoice");
        assert_eq!(doc.status(), DocumentStatus::Processing);

        let read = doc.with_content("text");
        assert_eq!(read.status(), DocumentStatus::NeedsReview);

        let filed = read.with_document_type(Id::new_v4());
        assert_eq!(filed.status(), DocumentStatus::Filed);
    }

    #[test]
    fn unreadable_documents_are_failed_not_waiting_for_review() {
        let mut doc = Document::new(VaultId::new(), "scan");
        doc.ocr_status = ProcessingStatus::Failed;
        assert_eq!(doc.status(), DocumentStatus::Failed);
    }
}
