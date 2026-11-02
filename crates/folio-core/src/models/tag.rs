//! Tag model

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Id, VaultId};

/// A tag for categorizing documents
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Tag {
    /// Unique identifier
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,

    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,

    /// Tag name (unique)
    pub name: String,

    /// Optional color for UI display (hex format, e.g., "#ff5733")
    pub color: Option<String>,

    /// When the tag was created
    pub created_at: DateTime<Utc>,
}

impl Tag {
    /// Create a new tag with the given name
    pub fn new(vault_id: VaultId, name: impl Into<String>) -> Self {
        Self {
            id: Id::new_v4(),
            vault_id,
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

    /// Palette tone the tag is drawn in. Derived from the id, so it never
    /// changes for the lifetime of a tag.
    pub fn tone(&self) -> &'static str {
        const TONES: [&str; 6] = ["sage", "slate", "clay", "ochre", "plum", "sky"];
        let sum: usize = self.id.as_bytes().iter().map(|b| *b as usize).sum();
        TONES[sum % TONES.len()]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_is_stable_and_in_the_palette() {
        let tag = Tag::new(VaultId::new(), "Insurance");
        assert_eq!(tag.tone(), tag.tone());
        assert!(["sage", "slate", "clay", "ochre", "plum", "sky"].contains(&tag.tone()));
    }

    #[test]
    fn tone_does_not_depend_on_name_or_color() {
        let tag = Tag::new(VaultId::new(), "a");
        let mut other = tag.clone();
        other.name = "b".into();
        other.color = Some("#fff".into());
        assert_eq!(tag.tone(), other.tone());
    }
}
