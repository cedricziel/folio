//! Domain events for real-time updates

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::{Document, Tag};

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
    /// Document was deleted
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

/// A [`DocumentEvent`] as persisted in the outbox
#[derive(Debug, Clone)]
pub struct OutboxEvent {
    /// Monotonic; also the SSE event id
    pub id: i64,
    pub event: DocumentEvent,
    pub origin: EventOrigin,
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
}

/// Encode an event for the outbox `payload` column
pub fn encode_outbox_payload(event: &DocumentEvent, origin: EventOrigin) -> String {
    let snapshot = match event {
        DocumentEvent::Deleted { snapshot, .. } => snapshot.as_deref().cloned(),
        _ => None,
    };
    serde_json::to_string(&StoredPayload {
        event: event.clone(),
        origin,
        snapshot,
    })
    .expect("document events always serialize")
}

/// Decode an outbox `payload` column
pub fn decode_outbox_payload(payload: &str) -> serde_json::Result<(DocumentEvent, EventOrigin)> {
    let StoredPayload {
        mut event,
        origin,
        snapshot,
    } = serde_json::from_str(payload)?;
    if let DocumentEvent::Deleted { snapshot: slot, .. } = &mut event {
        *slot = snapshot.map(Arc::new);
    }
    Ok((event, origin))
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
