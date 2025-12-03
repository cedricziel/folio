//! Tag model

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Id;

/// A tag for categorizing documents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    /// Unique identifier
    pub id: Id,

    /// Tag name (unique)
    pub name: String,

    /// Optional color for UI display (hex format, e.g., "#ff5733")
    pub color: Option<String>,

    /// When the tag was created
    pub created_at: DateTime<Utc>,
}

impl Tag {
    /// Create a new tag with the given name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Id::new_v4(),
            name: name.into(),
            color: None,
            created_at: Utc::now(),
        }
    }

    /// Set the tag color
    pub fn with_color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }
}

/// Parameters for creating a new tag
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTag {
    pub name: String,
    pub color: Option<String>,
}

/// Parameters for updating a tag
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateTag {
    pub name: Option<String>,
    pub color: Option<Option<String>>,
}
