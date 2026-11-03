use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Document, DocumentFilter, Id, VaultId};
use crate::error::{Error, Result};

/// Stable ordering for a document browse query.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DocumentSort {
    #[default]
    Newest,
    Oldest,
    Title,
}

/// A continuation bound to the vault and normalized query that produced it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentCursor {
    fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub title: String,
    pub id: Id,
}

/// A bounded query. Repositories validate it before reading any documents.
#[derive(Clone, Debug)]
pub struct DocumentPageQuery {
    pub filter: DocumentFilter,
    pub sort: DocumentSort,
    pub limit: u32,
    pub cursor: Option<DocumentCursor>,
}

impl Default for DocumentPageQuery {
    fn default() -> Self {
        Self {
            filter: DocumentFilter::default(),
            sort: DocumentSort::default(),
            limit: 50,
            cursor: None,
        }
    }
}

impl DocumentPageQuery {
    fn fingerprint(&self, vault: VaultId) -> String {
        let mut filter = self.filter.clone();
        if let Some(tags) = &mut filter.tag_ids {
            tags.sort();
            tags.dedup();
            if tags.is_empty() {
                filter.tag_ids = None;
            }
        }
        let bytes = serde_json::to_vec(&(1, vault, filter, self.sort))
            .expect("document query contains only serializable scalar values");
        hex::encode(Sha256::digest(bytes))
    }

    /// Reject invalid limits and continuations belonging to another query.
    pub fn validate(&self, vault: VaultId) -> Result<()> {
        self.filter.validate()?;
        if !(1..=100).contains(&self.limit) {
            return Err(Error::Validation(
                "Page limit must be between 1 and 100".into(),
            ));
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|c| c.fingerprint != self.fingerprint(vault))
        {
            return Err(Error::Validation(
                "Cursor does not match this document query".into(),
            ));
        }
        Ok(())
    }

    /// Build a page from at most limit + 1 ordered repository records.
    pub fn finish(&self, vault: VaultId, mut documents: Vec<Document>) -> DocumentPage {
        let has_more = documents.len() > self.limit as usize;
        documents.truncate(self.limit as usize);
        let next_cursor = documents
            .last()
            .filter(|_| has_more)
            .map(|last| DocumentCursor {
                fingerprint: self.fingerprint(vault),
                created_at: last.created_at,
                title: last.title.clone(),
                id: last.id,
            });
        DocumentPage {
            documents,
            next_cursor,
        }
    }
}

/// One bounded result set and its optional continuation.
#[derive(Debug)]
pub struct DocumentPage {
    pub documents: Vec<Document>,
    pub next_cursor: Option<DocumentCursor>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_survives_serialization_and_equivalent_tag_order() {
        let vault = VaultId::new();
        let a = Id::new_v4();
        let b = Id::new_v4();
        let query = DocumentPageQuery {
            limit: 1,
            filter: DocumentFilter {
                tag_ids: Some(vec![a, b]),
                ..Default::default()
            },
            ..Default::default()
        };
        let page = query.finish(
            vault,
            vec![Document::new(vault, "A"), Document::new(vault, "B")],
        );
        let encoded = serde_json::to_string(&page.next_cursor.unwrap()).unwrap();
        let continued = DocumentPageQuery {
            limit: 20,
            cursor: Some(serde_json::from_str(&encoded).unwrap()),
            filter: DocumentFilter {
                tag_ids: Some(vec![b, a, b]),
                ..Default::default()
            },
            ..query
        };
        continued.validate(vault).unwrap();
    }

    #[test]
    fn an_exactly_full_final_page_has_no_continuation() {
        let vault = VaultId::new();
        let query = DocumentPageQuery {
            limit: 1,
            ..Default::default()
        };
        let page = query.finish(vault, vec![Document::new(vault, "Only")]);
        assert_eq!(page.documents.len(), 1);
        assert!(page.next_cursor.is_none());
    }
}
