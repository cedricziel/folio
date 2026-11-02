//! Domain events for real-time updates

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::{Document, DocumentSource, Id, NamedRef, Tag, VaultId};

/// What a document looked like just before it was deleted
///
/// Workflows need it to evaluate conditions and build the payload after the
/// row is gone. It never goes over SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedDocument {
    pub document: Document,
    pub tags: Vec<Tag>,
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

/// A [`DocumentEvent`] as persisted in the outbox
#[derive(Debug, Clone)]
pub struct OutboxEvent {
    /// Monotonic; also the SSE event id
    pub id: i64,
    pub event: DocumentEvent,
    pub origin: EventOrigin,
    pub context: Option<Arc<EventSnapshot>>,
    pub created_at: DateTime<Utc>,
    /// Delivery attempts so far, including the current one
    pub attempts: i32,
}

/// What the outbox stores in the `payload` column
#[derive(Serialize, Deserialize)]
struct StoredPayload {
    event: DocumentEvent,
    origin: EventOrigin,
    snapshot: Option<DeletedDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<EventSnapshot>,
}

/// Encode an event for the outbox `payload` column
pub fn encode_outbox_payload(event: &DocumentEvent, origin: EventOrigin) -> String {
    encode_outbox_payload_with_context(event, origin, None)
}

/// Encode an event with its internal immutable metadata; wire events remain unchanged.
pub fn encode_outbox_payload_with_context(
    event: &DocumentEvent,
    origin: EventOrigin,
    context: Option<&EventSnapshot>,
) -> String {
    let snapshot = match event {
        DocumentEvent::Deleted { snapshot, .. } => snapshot.as_deref().cloned(),
        _ => None,
    };
    serde_json::to_string(&StoredPayload {
        event: event.clone(),
        origin,
        snapshot,
        context: context.cloned(),
    })
    .expect("document events always serialize")
}

/// Decode an outbox `payload` column
pub fn decode_outbox_payload(payload: &str) -> serde_json::Result<(DocumentEvent, EventOrigin)> {
    decode_outbox_payload_with_context(payload).map(|(event, origin, _)| (event, origin))
}

/// Decode both current context and older payloads that lack it.
pub fn decode_outbox_payload_with_context(
    payload: &str,
) -> serde_json::Result<(DocumentEvent, EventOrigin, Option<Arc<EventSnapshot>>)> {
    let StoredPayload {
        mut event,
        origin,
        snapshot,
        context,
    } = serde_json::from_str(payload)?;
    if let DocumentEvent::Deleted { snapshot: slot, .. } = &mut event {
        *slot = snapshot.map(Arc::new);
    }
    if context
        .as_ref()
        .is_some_and(|context| context.document_id != event.document_id())
    {
        return Err(<serde_json::Error as serde::de::Error>::custom(
            "Outbox context document ID does not match its event",
        ));
    }
    Ok((event, origin, context.map(Arc::new)))
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
