//! Server-Sent Events routes

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Router,
};
use futures::stream::Stream;
use futures::StreamExt;
use std::{convert::Infallible, time::Duration};
use tokio_stream::wrappers::BroadcastStream;

use crate::state::AppState;
use folio_core::events::DocumentEvent;

/// SSE frames for an event: its own, plus a legacy `updated` frame for `processed`
///
/// The web UI refreshes a document row on `updated`; processing used to end
/// with one, and older clients still rely on it.
fn sse_frames(event: &DocumentEvent) -> Vec<Event> {
    let Ok(data) = serde_json::to_string(event) else {
        return Vec::new();
    };
    let mut frames = vec![Event::default()
        .event(event.event_type())
        .data(data.clone())];
    if matches!(event, DocumentEvent::Processed { .. }) {
        frames.push(Event::default().event("updated").data(data));
    }
    frames
}

pub fn router() -> Router<AppState> {
    Router::new().route("/events", get(sse_handler))
}

/// Subscribe to real-time document events
///
/// Each SSE message has the event name from `DocumentEvent` (for example `created`)
/// and a JSON `DocumentEvent` as data. Event names: `created`, `updated`, `deleted`, `thumbnail_ready`,
/// `ocr_complete`, and `processed` (background processing finished). Comment-only keep-alive frames are sent every 30 seconds.
#[utoipa::path(
    get,
    path = "/events",
    operation_id = "streamEvents",
    tag = "events",
    responses(
        (status = 200, description = "Stream of document events", body = DocumentEvent, content_type = "text/event-stream"),
    )
)]
pub(crate) async fn sse_handler(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_broadcaster.subscribe();

    let stream = BroadcastStream::new(rx).flat_map(|result| {
        let frames: Vec<Result<Event, Infallible>> = match result {
            Ok(event) => sse_frames(&event).into_iter().map(Ok).collect(),
            Err(_) => Vec::new(), // Skip lagged messages
        };
        futures::stream::iter(frames)
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keep-alive"),
    )
}
