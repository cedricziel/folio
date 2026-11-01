//! Document model

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Id;

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

    /// State of the OCR stage
    pub ocr_status: ProcessingStatus,

    /// State of the embedding stage
    pub embed_status: ProcessingStatus,

    /// When the document was created in the system
    pub created_at: DateTime<Utc>,

    /// When the document was last updated
    pub updated_at: DateTime<Utc>,
}

impl Document {
    /// Create a new document with the given title
    pub fn new(title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Id::new_v4(),
            title: title.into(),
            content: None,
            original_filename: None,
            file_path: None,
            mime_type: None,
            correspondent_id: None,
            document_type_id: None,
            page_count: None,
            ocr_status: ProcessingStatus::Pending,
            embed_status: ProcessingStatus::Pending,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set the document content
    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
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

/// Parameters for creating a new document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDocument {
    pub title: String,
    pub content: Option<String>,
    pub original_filename: Option<String>,
    pub mime_type: Option<String>,
    pub correspondent_id: Option<Id>,
    pub document_type_id: Option<Id>,
}

/// Parameters for updating a document
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateDocument {
    pub title: Option<String>,
    pub content: Option<String>,
    pub correspondent_id: Option<Option<Id>>,
    pub document_type_id: Option<Option<Id>>,
}

/// Filter criteria for listing documents
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocumentFilter {
    pub correspondent_id: Option<Id>,
    pub document_type_id: Option<Id>,
    pub tag_ids: Option<Vec<Id>>,
    pub search_query: Option<String>,
}
