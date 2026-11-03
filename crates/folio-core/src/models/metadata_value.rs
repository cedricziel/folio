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
#[serde(deny_unknown_fields)]
pub struct MetadataChange {
    pub field_id: uuid::Uuid,
    /// Required: an explicit `null` clears, so a missing key cannot clear by accident.
    #[serde(deserialize_with = "Option::deserialize")]
    pub value: Option<MetadataValue>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_requires_an_explicit_value_or_null() {
        let field = uuid::Uuid::new_v4();
        assert!(
            serde_json::from_value::<MetadataChange>(serde_json::json!({"field_id": field}))
                .is_err()
        );
        let clear: MetadataChange =
            serde_json::from_value(serde_json::json!({"field_id": field, "value": null})).unwrap();
        assert_eq!(clear.value, None);
    }
}
