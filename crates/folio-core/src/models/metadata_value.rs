use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Id, MetadataKind, MetadataValue, UserId, VaultId};
use crate::{Error, Result};

/// A retained assignment; a cleared value keeps its lifecycle attribution with `value: None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DocumentMetadataValue {
    pub field_id: uuid::Uuid,
    pub value: Option<MetadataValue>,
    pub link_target: Option<MetadataLinkTarget>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub updated_by: Option<UserId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MetadataLinkTarget {
    pub title: String,
    pub trashed: bool,
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
    #[cfg_attr(feature = "openapi", schema(required = true))]
    pub value: Option<MetadataValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MetadataExactFilter {
    pub field_id: uuid::Uuid,
    pub value: MetadataValue,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataSearchCursor {
    pub after: Id,
    fingerprint: String,
}

pub struct MetadataSearchQuery {
    pub filter: MetadataExactFilter,
    pub limit: u32,
    pub cursor: Option<MetadataSearchCursor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MetadataSearchHit {
    pub id: uuid::Uuid,
    pub title: String,
}

pub struct MetadataSearchPage {
    pub documents: Vec<MetadataSearchHit>,
    pub next_cursor: Option<MetadataSearchCursor>,
}

impl MetadataSearchQuery {
    pub fn validate_limit(&self) -> Result<()> {
        if !(1..=100).contains(&self.limit) {
            return Err(Error::Validation(
                "Page limit must be between 1 and 100".into(),
            ));
        }
        Ok(())
    }

    pub fn canonical_value(&self, vault: VaultId, kind: MetadataKind) -> Result<MetadataValue> {
        let value = self.filter.value.clone().validate(kind, false)?;
        let fingerprint = self.fingerprint(vault, &value)?;
        if self
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.fingerprint != fingerprint)
        {
            return Err(Error::Validation(
                "Cursor does not match this metadata query".into(),
            ));
        }
        Ok(value)
    }

    fn fingerprint(&self, vault: VaultId, value: &MetadataValue) -> Result<String> {
        let bytes = serde_json::to_vec(&(1, vault, self.filter.field_id, value))
            .map_err(|error| Error::Repository(error.to_string()))?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }

    pub fn finish(
        &self,
        vault: VaultId,
        value: &MetadataValue,
        mut documents: Vec<MetadataSearchHit>,
    ) -> Result<MetadataSearchPage> {
        let has_more = documents.len() > self.limit as usize;
        documents.truncate(self.limit as usize);
        let next_cursor = documents
            .last()
            .filter(|_| has_more)
            .map(|last| -> Result<_> {
                Ok(MetadataSearchCursor {
                    after: last.id,
                    fingerprint: self.fingerprint(vault, value)?,
                })
            })
            .transpose()?;
        Ok(MetadataSearchPage {
            documents,
            next_cursor,
        })
    }
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
