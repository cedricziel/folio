use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::models::{Document, Id};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum DocumentChange {
    #[serde(rename = "document.title_changed")]
    TitleChanged { before: String, after: String },
    #[serde(rename = "document.correspondent_changed")]
    CorrespondentChanged {
        before_id: Option<Id>,
        after_id: Option<Id>,
    },
    #[serde(rename = "document.document_type_changed")]
    DocumentTypeChanged {
        before_id: Option<Id>,
        after_id: Option<Id>,
    },
    #[serde(rename = "document.tags_changed")]
    TagsChanged {
        before_ids: Vec<Id>,
        after_ids: Vec<Id>,
        added_ids: Vec<Id>,
        removed_ids: Vec<Id>,
    },
}

/// Compare metadata captured inside one mutation transaction; tag order and duplicates are ignored.
/// Callers retain responsibility for matching document identity and vault authorization.
pub fn document_changes(
    before: &Document,
    before_tags: &[Id],
    after: &Document,
    after_tags: &[Id],
) -> Vec<DocumentChange> {
    let mut changes = Vec::new();
    if before.title != after.title {
        changes.push(DocumentChange::TitleChanged {
            before: before.title.clone(),
            after: after.title.clone(),
        });
    }
    if before.correspondent_id != after.correspondent_id {
        changes.push(DocumentChange::CorrespondentChanged {
            before_id: before.correspondent_id,
            after_id: after.correspondent_id,
        });
    }
    if before.document_type_id != after.document_type_id {
        changes.push(DocumentChange::DocumentTypeChanged {
            before_id: before.document_type_id,
            after_id: after.document_type_id,
        });
    }
    let before_ids: BTreeSet<Id> = before_tags.iter().copied().collect();
    let after_ids: BTreeSet<Id> = after_tags.iter().copied().collect();
    if before_ids != after_ids {
        changes.push(DocumentChange::TagsChanged {
            before_ids: before_ids.iter().copied().collect(),
            after_ids: after_ids.iter().copied().collect(),
            added_ids: after_ids.difference(&before_ids).copied().collect(),
            removed_ids: before_ids.difference(&after_ids).copied().collect(),
        });
    }
    changes
}
