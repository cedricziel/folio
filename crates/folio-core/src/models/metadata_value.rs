use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{MetadataValue, UserId};

/// A retained assignment; a cleared value keeps its lifecycle attribution with `value: None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DocumentMetadataValue {
    pub field_id: uuid::Uuid,
    pub value: Option<MetadataValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub updated_by: Option<UserId>,
}

/// All retained values of one logical document, ordered by field ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DocumentMetadata {
    pub document_id: uuid::Uuid,
    /// Compare-and-swap token; `None` until the first metadata change.
    pub revision: Option<uuid::Uuid>,
    pub values: Vec<DocumentMetadataValue>,
}

/// One explicit replacement (`Some`) or clear (`None`); fields absent from a patch stay unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MetadataChange {
    pub field_id: uuid::Uuid,
    pub value: Option<MetadataValue>,
}
