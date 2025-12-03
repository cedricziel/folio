//! Document type model

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Id;

/// A document type for classification (e.g., Invoice, Receipt, Contract)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentType {
    /// Unique identifier
    pub id: Id,

    /// Type name (unique)
    pub name: String,

    /// When the document type was created
    pub created_at: DateTime<Utc>,
}

impl DocumentType {
    /// Create a new document type with the given name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Id::new_v4(),
            name: name.into(),
            created_at: Utc::now(),
        }
    }
}

/// Parameters for creating a new document type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDocumentType {
    pub name: String,
}

/// Parameters for updating a document type
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateDocumentType {
    pub name: Option<String>,
}
