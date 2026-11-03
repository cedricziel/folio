//! Domain events for real-time updates

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
use crate::models::{Document, DocumentDate, DocumentSource, Id, NamedRef, Tag, VaultId};

/// What a document looked like just before it was deleted
///
/// Workflows need it to evaluate conditions and build the payload after the
/// row is gone. It never goes over SSE.
#[derive(Debug, Clone, Serialize)]
pub struct DeletedDocument {
    pub document: Document,
    pub tags: Vec<Tag>,
}

impl<'de> Deserialize<'de> for DeletedDocument {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Snapshot {
            document: Document,
            tags: Vec<serde_json::Value>,
        }
        let snapshot = Snapshot::deserialize(deserializer)?;
        let tags = snapshot
            .tags
            .into_iter()
            .map(|mut tag| {
                // Older stored snapshots omitted tag ownership; the captured document supplies it.
                if let Some(fields) = tag.as_object_mut() {
                    fields.entry("vault_id").or_insert_with(|| {
                        serde_json::Value::String(snapshot.document.vault_id.to_string())
                    });
                }
                serde_json::from_value(tag).map_err(serde::de::Error::custom)
            })
            .collect::<Result<Vec<Tag>, D::Error>>()?;
        Ok(Self {
            document: snapshot.document,
            tags,
        })
    }
}

/// Event types that can be broadcast to clients
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocumentEvent {
    /// Document was created
    Created { document_id: Uuid },
    /// Document was updated (content, metadata, etc.)
    Updated { document_id: Uuid },
    /// Document thumbnail was generated
    ThumbnailReady { document_id: Uuid },
    /// Document OCR completed
    OcrComplete { document_id: Uuid },
    /// Document moved to trash.
    Trashed { document_id: Uuid },
    /// Permanent deletion was requested.
    PurgeRequested { document_id: Uuid },
    /// Document returned from trash.
    Restored { document_id: Uuid },
    /// Document was permanently deleted
    Deleted {
        document_id: Uuid,
        #[serde(skip)]
        #[cfg_attr(feature = "openapi", schema(ignore))]
        snapshot: Option<Arc<DeletedDocument>>,
    },
    /// Background processing (thumbnails, OCR, embedding) finished
    Processed { document_id: Uuid },
}

impl DocumentEvent {
    /// Get the document ID for this event
    pub fn document_id(&self) -> Uuid {
        match self {
            Self::Created { document_id }
            | Self::Updated { document_id }
            | Self::ThumbnailReady { document_id }
            | Self::OcrComplete { document_id }
            | Self::Trashed { document_id }
            | Self::PurgeRequested { document_id }
            | Self::Restored { document_id }
            | Self::Deleted { document_id, .. }
            | Self::Processed { document_id } => *document_id,
        }
    }

    /// Get the event type name
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::Created { .. } => "created",
            Self::Updated { .. } => "updated",
            Self::ThumbnailReady { .. } => "thumbnail_ready",
            Self::OcrComplete { .. } => "ocr_complete",
            Self::Trashed { .. } => "trashed",
            Self::PurgeRequested { .. } => "purge_requested",
            Self::Restored { .. } => "restored",
            Self::Deleted { .. } => "deleted",
            Self::Processed { .. } => "processed",
        }
    }
}

/// Who caused an event
///
/// Workflow edits are announced to live clients like any other change, but
/// must not trigger workflows again, or two workflows could feed each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventOrigin {
    User,
    Workflow,
}

/// Immutable metadata captured in the transaction that produced an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_date: Option<DocumentDate>,
    pub document_id: Id,
    pub vault_id: VaultId,
    pub title: String,
    pub original_filename: Option<String>,
    pub mime_type: Option<String>,
    pub file_path: Option<String>,
    pub source: DocumentSource,
    pub created_at: DateTime<Utc>,
    pub correspondent_id: Option<Id>,
    pub correspondent_name: Option<String>,
    pub document_type_id: Option<Id>,
    pub document_type_name: Option<String>,
    pub tags: Vec<NamedRef>,
}

impl From<&DeletedDocument> for EventSnapshot {
    fn from(snapshot: &DeletedDocument) -> Self {
        let doc = &snapshot.document;
        Self {
            document_date: doc.document_date,
            document_id: doc.id,
            vault_id: doc.vault_id,
            title: doc.title.clone(),
            original_filename: doc.original_filename.clone(),
            mime_type: doc.mime_type.clone(),
            file_path: doc.file_path.clone(),
            source: doc.source.clone(),
            created_at: doc.created_at,
            correspondent_id: doc.correspondent_id,
            correspondent_name: None,
            document_type_id: doc.document_type_id,
            document_type_name: None,
            tags: snapshot
                .tags
                .iter()
                .map(|tag| NamedRef {
                    id: tag.id,
                    name: tag.name.clone(),
                })
                .collect(),
        }
    }
}

/// A committed domain transition as persisted in the outbox.
#[derive(Debug, Clone)]
pub struct OutboxEvent {
    /// Monotonic delivery and SSE cursor.
    pub id: i64,
    pub event: Option<DocumentEvent>,
    pub envelope: Option<DomainEventEnvelope>,
    pub origin: EventOrigin,
    pub context: Option<Arc<EventSnapshot>>,
    pub created_at: DateTime<Utc>,
    pub attempts: i32,
}

impl OutboxEvent {
    /// The captured document origin, including legacy deletion snapshots and versioned envelopes.
    pub fn origin_vault(&self) -> Option<VaultId> {
        let event = self.event.as_ref()?;
        let vault = match &self.context {
            Some(context) => {
                (context.document_id == event.document_id()).then_some(context.vault_id)
            }
            None => match event {
                DocumentEvent::Deleted {
                    document_id,
                    snapshot: Some(snapshot),
                } if snapshot.document.id == *document_id => Some(snapshot.document.vault_id),
                _ => self.envelope.as_ref().and_then(|envelope| {
                    (envelope.document_id() == Some(event.document_id()))
                        .then_some(envelope.vault_id())
                }),
            },
        };
        vault.filter(|vault| {
            self.envelope
                .as_ref()
                .is_none_or(|envelope| envelope.vault_id() == *vault)
        })
    }
}

/// What the outbox stores in the `payload` column
#[derive(Serialize, Deserialize)]
struct StoredPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event: Option<DocumentEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    envelope: Option<DomainEventEnvelope>,
    origin: EventOrigin,
    snapshot: Option<DeletedDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<EventSnapshot>,
}

/// Decoded durable payload, including legacy document-only rows.
#[derive(Debug, Clone)]
pub struct DecodedOutboxPayload {
    pub event: Option<DocumentEvent>,
    pub envelope: Option<DomainEventEnvelope>,
    pub origin: EventOrigin,
    pub context: Option<Arc<EventSnapshot>>,
}

/// Encode a legacy document projection without changing its stored contract.
pub fn encode_outbox_payload(event: &DocumentEvent, origin: EventOrigin) -> String {
    encode_outbox_payload_with_context(event, origin, None)
}

/// Encode a legacy document projection and its immutable internal metadata.
pub fn encode_outbox_payload_with_context(
    event: &DocumentEvent,
    origin: EventOrigin,
    context: Option<&EventSnapshot>,
) -> String {
    serde_json::to_string(&stored_payload(Some(event), origin, context, None))
        .expect("document events always serialize")
}

fn stored_payload(
    event: Option<&DocumentEvent>,
    origin: EventOrigin,
    context: Option<&EventSnapshot>,
    envelope: Option<&DomainEventEnvelope>,
) -> StoredPayload {
    let snapshot = match event {
        Some(DocumentEvent::Deleted { snapshot, .. }) => snapshot.as_deref().cloned(),
        _ => None,
    };
    StoredPayload {
        event: event.cloned(),
        envelope: envelope.cloned(),
        origin,
        snapshot,
        context: context.cloned(),
    }
}

/// Encode a validated domain payload, optionally carrying its legacy document projection.
/// Neither empty payloads nor inconsistent document or vault identities are accepted.
pub fn encode_stored_outbox_payload(
    event: Option<&DocumentEvent>,
    origin: EventOrigin,
    context: Option<&EventSnapshot>,
    envelope: Option<&DomainEventEnvelope>,
) -> crate::Result<String> {
    let payload = serde_json::to_string(&stored_payload(event, origin, context, envelope))
        .map_err(|error| crate::Error::Validation(error.to_string()))?;
    decode_stored_outbox_payload(&payload)
        .map_err(|error| crate::Error::Validation(error.to_string()))?;
    Ok(payload)
}

/// Decode a legacy document projection. Domain-only rows have no document projection.
pub fn decode_outbox_payload(payload: &str) -> serde_json::Result<(DocumentEvent, EventOrigin)> {
    decode_outbox_payload_with_context(payload).map(|(event, origin, _)| (event, origin))
}

/// Decode document payloads with or without an immutable context and domain envelope.
pub fn decode_outbox_payload_with_context(
    payload: &str,
) -> serde_json::Result<(DocumentEvent, EventOrigin, Option<Arc<EventSnapshot>>)> {
    let decoded = decode_stored_outbox_payload(payload)?;
    let event = decoded.event.ok_or_else(|| {
        <serde_json::Error as serde::de::Error>::custom(
            "Domain-only outbox payload has no document projection",
        )
    })?;
    Ok((event, decoded.origin, decoded.context))
}

/// Decode legacy and versioned domain payloads while checking their scoped identities.
pub fn decode_stored_outbox_payload(payload: &str) -> serde_json::Result<DecodedOutboxPayload> {
    let StoredPayload {
        mut event,
        envelope,
        origin,
        snapshot,
        context,
    } = serde_json::from_str(payload)?;
    let invalid = |message: &str| <serde_json::Error as serde::de::Error>::custom(message);
    if event.is_none() && envelope.is_none() {
        return Err(invalid("Outbox payload requires an event or envelope"));
    }
    if let Some(DocumentEvent::Deleted { snapshot: slot, .. }) = &mut event {
        *slot = snapshot.map(Arc::new);
    }
    if let Some(event) = event.as_ref() {
        if envelope.as_ref().is_some_and(|envelope| {
            envelope.subject().subject_type != "document"
                || envelope.subject().id != event.document_id()
        }) {
            return Err(invalid(
                "Outbox document projection requires a matching document subject",
            ));
        }
        if context
            .as_ref()
            .is_some_and(|context| context.document_id != event.document_id())
        {
            return Err(invalid(
                "Outbox context document ID does not match its event",
            ));
        }
        if envelope
            .as_ref()
            .is_some_and(|envelope| envelope.document_id() != Some(event.document_id()))
        {
            return Err(invalid(
                "Outbox envelope document ID does not match its event",
            ));
        }
        if let DocumentEvent::Deleted {
            snapshot: Some(snapshot),
            ..
        } = event
        {
            if snapshot.document.id != event.document_id()
                || envelope
                    .as_ref()
                    .is_some_and(|envelope| envelope.vault_id() != snapshot.document.vault_id)
            {
                return Err(invalid("Outbox deleted snapshot does not match its event"));
            }
        }
    }
    if let (Some(context), Some(envelope)) = (context.as_ref(), envelope.as_ref()) {
        if context.vault_id != envelope.vault_id()
            || Some(context.document_id) != envelope.document_id()
        {
            return Err(invalid(
                "Outbox envelope does not match its captured context",
            ));
        }
    }
    Ok(DecodedOutboxPayload {
        event,
        envelope,
        origin,
        context: context.map(Arc::new),
    })
}

/// Capture the allowlisted document metadata for a new durable envelope.
/// Missing documents retain their legacy projection without inventing vault ownership.
pub fn document_event_envelope(
    event: &DocumentEvent,
    context: Option<&EventSnapshot>,
) -> crate::Result<Option<DomainEventEnvelope>> {
    let captured;
    let context = match context {
        Some(context) => context,
        None => {
            let DocumentEvent::Deleted {
                snapshot: Some(snapshot),
                ..
            } = event
            else {
                return Ok(None);
            };
            let document = &snapshot.document;
            captured = EventSnapshot {
                document_date: document.document_date,
                document_id: document.id,
                vault_id: document.vault_id,
                title: document.title.clone(),
                original_filename: document.original_filename.clone(),
                mime_type: document.mime_type.clone(),
                file_path: None,
                source: document.source.clone(),
                created_at: document.created_at,
                correspondent_id: document.correspondent_id,
                correspondent_name: None,
                document_type_id: document.document_type_id,
                document_type_name: None,
                tags: snapshot
                    .tags
                    .iter()
                    .map(|tag| NamedRef {
                        id: tag.id,
                        name: tag.name.clone(),
                    })
                    .collect(),
            };
            &captured
        }
    };
    DomainEventEnvelope::new(
        format!("document.{}", event.event_type()), context.vault_id,
        DomainEventSubject { subject_type: "document".into(), id: event.document_id() },
        Some(event.document_id()), EventAudience::Vault,
        serde_json::json!({
            "title": context.title, "original_filename": context.original_filename,
            "mime_type": context.mime_type, "created_at": context.created_at, "document_date": context.document_date,
            "correspondent_id": context.correspondent_id, "correspondent_name": context.correspondent_name,
            "document_type_id": context.document_type_id, "document_type_name": context.document_type_name,
            "tags": context.tags,
        }),
    ).map(Some)
}

/// In-process fan-out of dispatched events to live SSE connections
///
/// Delivery to this channel is best effort: it is not the source of truth.
/// Durable consumers read the outbox, and reconnecting clients replay from it.
#[derive(Clone)]
pub struct EventBroadcaster {
    sender: broadcast::Sender<OutboxEvent>,
}

impl EventBroadcaster {
    /// Create a new event broadcaster
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Send an event to all subscribers
    pub fn send(&self, event: OutboxEvent) {
        // Ignore send errors (no subscribers)
        let _ = self.sender.send(event);
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<OutboxEvent> {
        self.sender.subscribe()
    }
}

impl Default for EventBroadcaster {
    fn default() -> Self {
        Self::new(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> EventSnapshot {
        EventSnapshot {
            document_date: None,
            document_id: Id::new_v4(),
            vault_id: VaultId::new(),
            title: "at occurrence".into(),
            original_filename: Some("original.pdf".into()),
            mime_type: Some("application/pdf".into()),
            file_path: Some("immutable/original".into()),
            source: DocumentSource::default(),
            created_at: Utc::now(),
            correspondent_id: None,
            correspondent_name: None,
            document_type_id: None,
            document_type_name: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn immutable_context_round_trips_without_changing_the_wire_event() {
        let context = context();
        let event = DocumentEvent::Created {
            document_id: context.document_id,
        };
        let payload =
            encode_outbox_payload_with_context(&event, EventOrigin::Workflow, Some(&context));
        let (decoded, origin, restored) = decode_outbox_payload_with_context(&payload).unwrap();
        assert_eq!(origin, EventOrigin::Workflow);
        assert_eq!(restored.as_deref(), Some(&context));
        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            serde_json::json!({"type":"created", "document_id": context.document_id})
        );
    }

    #[test]
    fn mismatched_context_cannot_be_used_as_the_events_document() {
        let event = DocumentEvent::Updated {
            document_id: Id::new_v4(),
        };
        let payload =
            encode_outbox_payload_with_context(&event, EventOrigin::User, Some(&context()));
        assert!(decode_outbox_payload_with_context(&payload)
            .unwrap_err()
            .to_string()
            .contains("does not match"));
    }

    #[test]
    fn legacy_nondeletion_payload_has_no_inferred_context() {
        let event = DocumentEvent::Updated {
            document_id: Id::new_v4(),
        };
        let payload = encode_outbox_payload(&event, EventOrigin::User);
        assert!(decode_outbox_payload_with_context(&payload)
            .unwrap()
            .2
            .is_none());
    }

    #[test]
    fn legacy_document_payload_has_no_invented_domain_identity() {
        let event = DocumentEvent::Updated {
            document_id: Id::new_v4(),
        };
        let payload = encode_outbox_payload(&event, EventOrigin::User);
        let decoded = decode_stored_outbox_payload(&payload).unwrap();
        assert_eq!(decoded.event.unwrap().document_id(), event.document_id());
        assert!(decoded.envelope.is_none());
    }

    #[test]
    fn domain_only_payload_round_trips_without_a_document_projection() {
        let envelope = DomainEventEnvelope::new(
            "tag.created",
            VaultId::new(),
            DomainEventSubject {
                subject_type: "tag".into(),
                id: Id::new_v4(),
            },
            None,
            EventAudience::Vault,
            serde_json::json!({"name": "archive"}),
        )
        .unwrap();
        let payload =
            encode_stored_outbox_payload(None, EventOrigin::User, None, Some(&envelope)).unwrap();
        let decoded = decode_stored_outbox_payload(&payload).unwrap();
        assert!(decoded.event.is_none());
        assert_eq!(decoded.envelope, Some(envelope));
        assert!(decode_outbox_payload(&payload).is_err());
    }

    #[test]
    fn empty_payload_and_mismatched_envelope_are_rejected() {
        assert!(decode_stored_outbox_payload(r#"{"origin":"user","snapshot":null}"#).is_err());
        let context = context();
        let event = DocumentEvent::Updated {
            document_id: context.document_id,
        };
        let envelope = document_event_envelope(&event, Some(&context))
            .unwrap()
            .unwrap();
        let mut payload: serde_json::Value = serde_json::from_str(
            &encode_stored_outbox_payload(
                Some(&event),
                EventOrigin::User,
                Some(&context),
                Some(&envelope),
            )
            .unwrap(),
        )
        .unwrap();
        payload["envelope"]["vault_id"] = serde_json::json!(VaultId::new());
        assert!(decode_stored_outbox_payload(&payload.to_string()).is_err());
    }

    #[test]
    fn child_domain_event_cannot_imply_a_legacy_document_workflow_trigger() {
        let document_id = Id::new_v4();
        let envelope = DomainEventEnvelope::new(
            "comment.created",
            VaultId::new(),
            DomainEventSubject {
                subject_type: "comment".into(),
                id: Id::new_v4(),
            },
            Some(document_id),
            EventAudience::Vault,
            serde_json::json!({}),
        )
        .unwrap();
        let event = DocumentEvent::Updated { document_id };
        assert!(encode_stored_outbox_payload(
            Some(&event),
            EventOrigin::User,
            None,
            Some(&envelope)
        )
        .is_err());
    }

    #[test]
    fn legacy_deleted_snapshot_inherits_tag_vault_from_document() {
        let document = Document::new(VaultId::new(), "retained history");
        let tag = Tag::new(document.vault_id, "archive");
        let mut payload = serde_json::json!({
            "document": document,
            "tags": [tag]
        });
        payload["tags"][0]
            .as_object_mut()
            .unwrap()
            .remove("vault_id");
        let snapshot: DeletedDocument = serde_json::from_value(payload).unwrap();
        assert_eq!(snapshot.tags[0].vault_id, document.vault_id);
        assert_eq!(snapshot.tags[0].id, tag.id);
    }

    #[test]
    fn deleted_snapshot_survives_the_outbox_payload() {
        let document = Document::new(crate::models::VaultId::new(), "gone");
        let id = document.id;
        let event = DocumentEvent::Deleted {
            document_id: id,
            snapshot: Some(Arc::new(DeletedDocument {
                document,
                tags: Vec::new(),
            })),
        };
        let payload = encode_outbox_payload(&event, EventOrigin::Workflow);
        let (decoded, origin) = decode_outbox_payload(&payload).unwrap();
        assert_eq!(origin, EventOrigin::Workflow);
        let DocumentEvent::Deleted {
            document_id,
            snapshot,
        } = decoded
        else {
            panic!("expected a deleted event");
        };
        assert_eq!(document_id, id);
        assert_eq!(snapshot.unwrap().document.title, "gone");
    }
}
