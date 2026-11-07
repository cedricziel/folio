//! Documents carried over from another document system

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{DocumentDate, EntryBody, Id, MetadataValue, ProcessingStatus};

/// A document system Folio imports from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    Paperless,
}

impl ImportSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paperless => "paperless",
        }
    }
}

/// The identity a document had in the system it was imported from; unique per vault
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExternalRef {
    pub source: ImportSource,
    pub id: String,
}

/// A note carried over with its original creation time
#[derive(Debug, Clone)]
pub struct ImportedNote {
    /// The note's ID in the source system; unique per document
    pub external_id: String,
    pub body: EntryBody,
    pub created_at: DateTime<Utc>,
}

/// Everything written for one imported document, in one transaction
///
/// The file must already be stored at `file_path`. The document is created
/// by the current actor, never runs OCR, and is filed unless `in_inbox`.
#[derive(Debug, Clone)]
pub struct ImportedDocument {
    /// Derived from the vault and `external_ref`, so every retry agrees on it
    pub id: Id,
    pub external_ref: ExternalRef,
    pub title: String,
    /// The text from the other system; embeddings are skipped without it
    pub content: Option<String>,
    pub original_filename: Option<String>,
    pub mime_type: Option<String>,
    pub file_path: String,
    /// Lowercase hex SHA-256 of the stored original
    pub content_sha256: String,
    pub document_date: Option<DocumentDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub correspondent_id: Option<Id>,
    pub document_type_id: Option<Id>,
    pub tag_ids: Vec<Id>,
    /// Already validated against each field's kind; keyed by field
    pub metadata: BTreeMap<Id, MetadataValue>,
    pub notes: Vec<ImportedNote>,
    /// The document carried an inbox tag, so it stays unreviewed
    pub in_inbox: bool,
}

impl ImportedDocument {
    /// Reject what the database would only report mid-transaction
    pub fn validate(&self) -> crate::Result<()> {
        let mut seen = std::collections::HashSet::new();
        if !self.notes.iter().all(|note| seen.insert(&note.external_id)) {
            return Err(crate::Error::Validation(
                "Imported notes must have distinct external IDs".into(),
            ));
        }
        Ok(())
    }

    /// Embeddings run only for documents that have text
    pub fn embed_status(&self) -> ProcessingStatus {
        if self
            .content
            .as_deref()
            .is_some_and(|c| !c.trim().is_empty())
        {
            ProcessingStatus::Pending
        } else {
            ProcessingStatus::Skipped
        }
    }
}
