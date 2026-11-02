use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{DocumentCursor, DocumentFilter, DocumentPageQuery, DocumentSort, Id, UserId, VaultId};
use crate::error::{Error, Result};

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewVisibility {
    #[default]
    Private,
    Shared,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewPresentation {
    #[default]
    List,
    Grid,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedViewQuery {
    pub version: u32,
    #[serde(default)]
    pub filters: DocumentFilter,
    #[serde(default)]
    pub sort: DocumentSort,
}

impl Default for SavedViewQuery {
    fn default() -> Self {
        Self {
            version: 1,
            filters: DocumentFilter::default(),
            sort: DocumentSort::default(),
        }
    }
}

impl SavedViewQuery {
    /// Validate the stored query and bind its continuation to the current vault.
    pub fn page(
        &self,
        vault: VaultId,
        limit: u32,
        cursor: Option<DocumentCursor>,
    ) -> Result<DocumentPageQuery> {
        if self.version != 1 {
            return Err(Error::Validation(
                "Unsupported saved-view query version".into(),
            ));
        }
        if self
            .filters
            .created_after
            .zip(self.filters.created_before)
            .is_some_and(|(after, before)| after >= before)
        {
            return Err(Error::Validation(
                "The end date must follow the start date".into(),
            ));
        }
        if self
            .filters
            .tag_ids
            .as_ref()
            .is_some_and(|tags| tags.len() > 100)
        {
            return Err(Error::Validation("At most 100 tags are supported".into()));
        }
        if self
            .filters
            .search_query
            .as_ref()
            .is_some_and(|text| text.len() > 4096 || text.split_whitespace().count() > 32)
        {
            return Err(Error::Validation(
                "Search supports at most 32 words and 4096 bytes".into(),
            ));
        }
        let query = DocumentPageQuery {
            filter: self.filters.clone(),
            sort: self.sort,
            limit,
            cursor,
        };
        query.validate(vault)?;
        Ok(query)
    }
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedViewInput {
    pub name: String,
    #[serde(default)]
    pub visibility: ViewVisibility,
    pub query: SavedViewQuery,
    #[serde(default)]
    pub presentation: ViewPresentation,
}

impl SavedViewInput {
    /// Validate editable properties without accepting ownership or audit fields.
    pub fn validate(&self, vault: VaultId) -> Result<()> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
            return Err(Error::Validation(
                "Name must contain 1 to 100 characters without control characters".into(),
            ));
        }
        self.query.page(vault, 50, None)?;
        Ok(())
    }
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedView {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub owner_id: UserId,
    pub name: String,
    pub visibility: ViewVisibility,
    pub query: SavedViewQuery,
    pub presentation: ViewPresentation,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub updated_by: Option<UserId>,
}

impl SavedView {
    /// Private views remain owner-only, even for vault owners.
    pub fn visible_to(&self, user: &UserId) -> bool {
        self.deleted_at.is_none()
            && (self.owner_id == *user || self.visibility == ViewVisibility::Shared)
    }

    /// Shared-view owners need Editor access; vault Owners may manage shared views.
    pub fn editable_by(&self, user: &UserId, role: super::VaultRole) -> bool {
        self.visible_to(user)
            && match self.visibility {
                ViewVisibility::Private => self.owner_id == *user,
                ViewVisibility::Shared => {
                    role == super::VaultRole::Owner
                        || (self.owner_id == *user && role >= super::VaultRole::Editor)
                }
            }
    }
}
