use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{MetadataKind, UserId, VaultId};
use crate::{Error, Result};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MetadataField {
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub name: String,
    pub kind: MetadataKind,
    pub required: bool,
    pub unique: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub updated_by: Option<UserId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MetadataOption {
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub field_id: uuid::Uuid,
    pub label: String,
    pub position: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub updated_by: Option<UserId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MetadataFieldInput {
    pub name: String,
    pub kind: MetadataKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub unique: bool,
}

impl MetadataFieldInput {
    /// Trim the name and reject blank names or names longer than 128 Unicode scalar values.
    pub fn validate(mut self) -> Result<Self> {
        self.name = validate_label(&self.name)?;
        Ok(self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MetadataOptionInput {
    pub label: String,
    #[serde(default)]
    pub position: u32,
}

impl MetadataOptionInput {
    /// Trim the label without changing option identity or ordering.
    pub fn validate(mut self) -> Result<Self> {
        self.label = validate_label(&self.label)?;
        Ok(self)
    }
}

fn validate_label(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 128 {
        return Err(Error::Validation(
            "Field names and option labels must contain 1 to 128 characters".into(),
        ));
    }
    Ok(value.to_owned())
}
