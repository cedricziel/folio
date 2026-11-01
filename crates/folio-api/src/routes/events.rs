//! Server-Sent Events routes

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Router,
};
use futures::stream::Stream;
use std::{convert::Infallible, time::Duration};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::state::AppState;
use folio_core::events::DocumentEvent;

pub fn router() -> Router<AppState> {
    Router::new().route("/events", get(sse_handler))
}

/// Subscribe to real-time document events
///
/// Each SSE message has the event name from `DocumentEvent` (for example `created`)
/// and a JSON `DocumentEvent` as data. Comment-only keep-alive frames are sent every 30 seconds.
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

    let stream = BroadcastStream::new(rx).filter_map(|result| {
        match result {
            Ok(event) => {
                let event_type = event.event_type();
                match serde_json::to_string(&event) {
                    Ok(data) => Some(Ok(Event::default().event(event_type).data(data))),
                    Err(_) => None,
                }
            }
            Err(_) => None, // Skip lagged messages
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keep-alive"),
    )
}
