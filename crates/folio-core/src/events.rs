//! Domain events for real-time updates

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::{Document, Tag};

/// What a document looked like just before it was deleted
///
/// Workflows need it to evaluate conditions and build the payload after the
/// row is gone. It never goes over SSE.
#[derive(Debug, Clone)]
pub struct DeletedDocument {
    pub document: Document,
    pub tags: Vec<Tag>,
}

/// Event types that can be broadcast to clients
#[derive(Debug, Clone, Serialize)]
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

/// Event broadcaster for SSE
#[derive(Clone)]
pub struct EventBroadcaster {
    sender: broadcast::Sender<DocumentEvent>,
}

impl EventBroadcaster {
    /// Create a new event broadcaster
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Send an event to all subscribers
    pub fn send(&self, event: DocumentEvent) {
        // Ignore send errors (no subscribers)
        let _ = self.sender.send(event);
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<DocumentEvent> {
        self.sender.subscribe()
    }
}

impl Default for EventBroadcaster {
    fn default() -> Self {
        Self::new(100)
    }
}
