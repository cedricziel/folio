//! Correspondent model

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Id;

/// A correspondent (sender/source) of documents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Correspondent {
    /// Unique identifier
    pub id: Id,

    /// Correspondent name (unique)
    pub name: String,

    /// When the correspondent was created
    pub created_at: DateTime<Utc>,
}

impl Correspondent {
    /// Create a new correspondent with the given name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Id::new_v4(),
            name: name.into(),
            created_at: Utc::now(),
        }
    }
}

/// Parameters for creating a new correspondent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCorrespondent {
    pub name: String,
}

/// Parameters for updating a correspondent
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateCorrespondent {
    pub name: Option<String>,
}
