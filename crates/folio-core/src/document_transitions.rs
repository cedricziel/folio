use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
use crate::events::{document_event_envelope, DocumentEvent, EventSnapshot};
use crate::models::{Document, DocumentDate, Id};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum DocumentChange {
    #[serde(rename = "document.date_changed")]
    DateChanged {
        previous_document_date: Option<DocumentDate>,
        document_date: Option<DocumentDate>,
    },
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
    compare_metadata(
        Metadata {
            document_date: before.document_date,
            title: &before.title,
            correspondent_id: before.correspondent_id,
            document_type_id: before.document_type_id,
        },
        before_tags,
        Metadata {
            document_date: after.document_date,
            title: &after.title,
            correspondent_id: after.correspondent_id,
            document_type_id: after.document_type_id,
        },
        after_tags,
    )
}

struct Metadata<'a> {
    document_date: Option<DocumentDate>,
    title: &'a str,
    correspondent_id: Option<Id>,
    document_type_id: Option<Id>,
}

fn compare_metadata(
    before: Metadata<'_>,
    before_tags: &[Id],
    after: Metadata<'_>,
    after_tags: &[Id],
) -> Vec<DocumentChange> {
    let mut changes = Vec::new();
    if before.document_date != after.document_date {
        changes.push(DocumentChange::DateChanged {
            previous_document_date: before.document_date,
            document_date: after.document_date,
        });
    }
    if before.title != after.title {
        changes.push(DocumentChange::TitleChanged {
            before: before.title.to_owned(),
            after: after.title.to_owned(),
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

/// Build specific deltas and one compatibility update for an actual committed metadata/content edit.
/// Call inside the mutation transaction after suppressing no-ops. Snapshots must describe the same
/// document and vault; all returned events share mutation and causal identities.
pub fn metadata_envelopes(
    before: &EventSnapshot,
    after: &EventSnapshot,
) -> Result<Vec<DomainEventEnvelope>> {
    if before.document_id != after.document_id || before.vault_id != after.vault_id {
        return Err(Error::Validation(
            "Metadata transitions require the same document and vault".into(),
        ));
    }
    let generic = document_event_envelope(
        &DocumentEvent::Updated {
            document_id: after.document_id,
        },
        Some(after),
    )?
    .ok_or_else(|| Error::Validation("Metadata transition has no occurrence snapshot".into()))?;
    let before_tags: Vec<Id> = before.tags.iter().map(|tag| tag.id).collect();
    let after_tags: Vec<Id> = after.tags.iter().map(|tag| tag.id).collect();
    let changes = compare_metadata(
        Metadata {
            document_date: before.document_date,
            title: &before.title,
            correspondent_id: before.correspondent_id,
            document_type_id: before.document_type_id,
        },
        &before_tags,
        Metadata {
            document_date: after.document_date,
            title: &after.title,
            correspondent_id: after.correspondent_id,
            document_type_id: after.document_type_id,
        },
        &after_tags,
    );
    let mut events = Vec::new();
    for change in changes {
        let mut delta =
            serde_json::to_value(change).map_err(|error| Error::Repository(error.to_string()))?;
        let event_type = delta
            .as_object_mut()
            .and_then(|fields| fields.remove("event_type"))
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(|| Error::Validation("Document delta has no event type".into()))?;
        let mut data = generic.data().clone();
        data.as_object_mut()
            .ok_or_else(|| Error::Validation("Document snapshot must be an object".into()))?
            .extend(
                delta
                    .as_object()
                    .ok_or_else(|| Error::Validation("Document delta must be an object".into()))?
                    .clone(),
            );
        data["source"] = serde_json::to_value(&after.source)
            .map_err(|error| Error::Repository(error.to_string()))?;
        let event = DomainEventEnvelope::new(
            event_type,
            after.vault_id,
            DomainEventSubject {
                subject_type: "document".into(),
                id: after.document_id,
            },
            Some(after.document_id),
            EventAudience::Vault,
            data,
        )?
        .with_mutation(generic.mutation_id())?
        .with_correlation(generic.correlation_id(), generic.causation_id())?;
        events.push(event);
    }
    events.push(generic);
    Ok(events)
}
