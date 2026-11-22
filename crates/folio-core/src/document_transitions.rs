use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
use crate::events::{document_event_envelope, DocumentEvent, EventSnapshot};
use crate::models::{Document, DocumentDate, FileIdentity, Id, ReviewOrigin, UserId};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum DocumentChange {
    #[serde(rename = "document.reviewed")]
    Reviewed {
        reviewed_at: DateTime<Utc>,
        reviewed_by: Option<UserId>,
        review_origin: Option<ReviewOrigin>,
    },
    #[serde(rename = "document.unreviewed")]
    Unreviewed {
        previous_reviewed_at: DateTime<Utc>,
        previous_reviewed_by: Option<UserId>,
        previous_review_origin: Option<ReviewOrigin>,
        reason: String,
    },
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
            reviewed_at: before.reviewed_at,
            reviewed_by: before.reviewed_by.as_ref(),
            review_origin: before.review_origin,
            title: &before.title,
            correspondent_id: before.correspondent_id,
            document_type_id: before.document_type_id,
        },
        before_tags,
        Metadata {
            document_date: after.document_date,
            reviewed_at: after.reviewed_at,
            reviewed_by: after.reviewed_by.as_ref(),
            review_origin: after.review_origin,
            title: &after.title,
            correspondent_id: after.correspondent_id,
            document_type_id: after.document_type_id,
        },
        after_tags,
    )
}

struct Metadata<'a> {
    reviewed_at: Option<DateTime<Utc>>,
    reviewed_by: Option<&'a UserId>,
    review_origin: Option<ReviewOrigin>,
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
    match (before.reviewed_at, after.reviewed_at) {
        (None, Some(reviewed_at)) => changes.push(DocumentChange::Reviewed {
            reviewed_at,
            reviewed_by: after.reviewed_by.cloned(),
            review_origin: after.review_origin,
        }),
        (Some(previous_reviewed_at), None) => changes.push(DocumentChange::Unreviewed {
            previous_reviewed_at,
            previous_reviewed_by: before.reviewed_by.cloned(),
            previous_review_origin: before.review_origin,
            reason: if crate::mutation::current_actor().is_some() {
                "user"
            } else {
                "system"
            }
            .into(),
        }),
        _ => {}
    }
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
    metadata_envelopes_with_unreview_reason(before, after, None)
}

fn metadata_envelopes_with_unreview_reason(
    before: &EventSnapshot,
    after: &EventSnapshot,
    unreview_reason: Option<&str>,
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
            reviewed_at: before.reviewed_at,
            reviewed_by: before.reviewed_by.as_ref(),
            review_origin: before.review_origin,
            title: &before.title,
            correspondent_id: before.correspondent_id,
            document_type_id: before.document_type_id,
        },
        &before_tags,
        Metadata {
            document_date: after.document_date,
            reviewed_at: after.reviewed_at,
            reviewed_by: after.reviewed_by.as_ref(),
            review_origin: after.review_origin,
            title: &after.title,
            correspondent_id: after.correspondent_id,
            document_type_id: after.document_type_id,
        },
        &after_tags,
    );
    let mut events = Vec::new();
    for mut change in changes {
        if let (DocumentChange::Unreviewed { reason, .. }, Some(revision_reason)) =
            (&mut change, unreview_reason)
        {
            *reason = revision_reason.into();
        }
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

#[derive(Debug, Clone, Copy)]
pub struct RevisionOccurrence<'a> {
    pub revision_id: Id,
    pub revision_number: i64,
    pub previous_revision_id: Option<Id>,
    pub restored_from_revision_id: Option<Id>,
    pub original: &'a FileIdentity,
}

/// Build the first revision occurrence and preserve the upload's identity and lineage.
pub fn first_revision_envelope(
    uploaded: &DomainEventEnvelope,
    document: &EventSnapshot,
    revision_id: Id,
    original: &FileIdentity,
) -> Result<DomainEventEnvelope> {
    revision_envelope(
        "document.revision_created",
        uploaded,
        document,
        RevisionOccurrence {
            revision_id,
            revision_number: 1,
            previous_revision_id: None,
            restored_from_revision_id: None,
            original,
        },
    )
}

/// Build occurrences for one successful replacement or restore transaction.
pub fn revision_publication_envelopes(
    before: &EventSnapshot,
    after: &EventSnapshot,
    occurrence: RevisionOccurrence<'_>,
) -> Result<Vec<DomainEventEnvelope>> {
    let Some(previous_revision_id) = occurrence.previous_revision_id else {
        return Err(Error::Validation(
            "A revision publication requires a previous revision".into(),
        ));
    };
    if occurrence.revision_number <= 1
        || previous_revision_id == occurrence.revision_id
        || occurrence.restored_from_revision_id.is_some_and(|source| {
            source == occurrence.revision_id || source == previous_revision_id
        })
        || after.reviewed_at.is_some()
        || after.reviewed_by.is_some()
        || after.review_origin.is_some()
    {
        return Err(Error::Validation(
            "Invalid revision publication snapshot".into(),
        ));
    }

    let restoring = occurrence.restored_from_revision_id.is_some();
    let reason = if restoring {
        "file_restored"
    } else {
        "file_replaced"
    };
    let mut events = metadata_envelopes_with_unreview_reason(before, after, Some(reason))?;
    let updated = events
        .pop()
        .ok_or_else(|| Error::Validation("Revision publication has no update occurrence".into()))?;
    events.push(revision_envelope(
        "document.revision_created",
        &updated,
        after,
        occurrence,
    )?);
    if restoring {
        events.push(revision_envelope(
            "document.revision_restored",
            &updated,
            after,
            occurrence,
        )?);
    }
    events.push(updated);
    Ok(events)
}

fn revision_envelope(
    event_type: &str,
    previous: &DomainEventEnvelope,
    document: &EventSnapshot,
    occurrence: RevisionOccurrence<'_>,
) -> Result<DomainEventEnvelope> {
    let mut data = previous.data().clone();
    data["source"] = serde_json::to_value(&document.source)
        .map_err(|error| Error::Repository(error.to_string()))?;
    data["revision_id"] = serde_json::json!(occurrence.revision_id);
    data["revision_number"] = serde_json::json!(occurrence.revision_number);
    data["previous_revision_id"] = serde_json::json!(occurrence.previous_revision_id);
    data["restored_from_revision_id"] = serde_json::json!(occurrence.restored_from_revision_id);
    data["mime_type"] = serde_json::json!(occurrence.original.mime_type);
    data["byte_size"] = serde_json::json!(occurrence.original.byte_size);
    data["sha256"] = serde_json::json!(occurrence.original.sha256);
    DomainEventEnvelope::new(
        event_type,
        document.vault_id,
        DomainEventSubject {
            subject_type: "document".into(),
            id: document.document_id,
        },
        Some(document.document_id),
        EventAudience::Vault,
        data,
    )?
    .with_mutation(previous.mutation_id())?
    .with_correlation(previous.correlation_id(), previous.causation_id())
}

/// Capture each side of a move without exposing the other vault's identifiers.
pub fn move_envelopes(
    before: &EventSnapshot,
    after: &EventSnapshot,
    mutation_id: Id,
) -> Result<[DomainEventEnvelope; 2]> {
    if before.document_id != after.document_id || before.vault_id == after.vault_id {
        return Err(Error::Validation(
            "Move snapshots require one document in distinct vaults".into(),
        ));
    }
    let build = |snapshot: &EventSnapshot, event_type: &str| -> Result<DomainEventEnvelope> {
        let generic = document_event_envelope(
            &DocumentEvent::Updated {
                document_id: snapshot.document_id,
            },
            Some(snapshot),
        )?
        .ok_or_else(|| Error::Validation("Move has no occurrence snapshot".into()))?;
        let mut data = generic.data().clone();
        data["source"] = serde_json::to_value(&snapshot.source)
            .map_err(|error| Error::Repository(error.to_string()))?;
        DomainEventEnvelope::new(
            event_type,
            snapshot.vault_id,
            DomainEventSubject {
                subject_type: "document".into(),
                id: snapshot.document_id,
            },
            Some(snapshot.document_id),
            EventAudience::Vault,
            data,
        )?
        .with_mutation(mutation_id)
    };
    let outgoing = build(before, "document.moved_out")?;
    let incoming = build(after, "document.moved_in")?
        .with_correlation(outgoing.correlation_id(), outgoing.causation_id())?;
    Ok([outgoing, incoming])
}
