//! Server-Sent Events routes

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Router,
};
use futures::stream::{Stream, StreamExt as _};
use std::{convert::Infallible, time::Duration};
use tokio_stream::wrappers::BroadcastStream;

use crate::error::ApiResult;
use crate::state::AppState;
use folio_core::events::DocumentEvent;

use super::pages::DocumentStreams;

pub fn router() -> Router<AppState> {
    Router::new().route("/events", get(sse_handler))
}

/// Turbo Streams that bring open pages up to date after `event`.
pub async fn turbo_streams_for(state: &AppState, event: &DocumentEvent) -> ApiResult<String> {
    let streams = match event {
        DocumentEvent::Deleted { document_id } => {
            DocumentStreams::removed(state, *document_id).await?
        }
        _ => {
            let Some(document) = state.document_repo.find_by_id(event.document_id()).await? else {
                return Ok(String::new());
            };
            match event {
                DocumentEvent::Created { .. } => DocumentStreams::added(state, document).await?,
                _ => DocumentStreams::changed(state, document).await?,
            }
        }
    };
    Ok(streams.into_html())
}

/// Subscribe to real-time document events
///
/// Each SSE message has the event name from `DocumentEvent` (for example `created`)
/// and a JSON `DocumentEvent` as data. Each event is followed by an unnamed message holding the
/// Turbo Streams that update the web UI. Comment-only keep-alive frames are sent every 30 seconds.
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

    let stream = BroadcastStream::new(rx).filter_map(move |result| {
        let state = state.clone();
        async move {
            // Lagged messages are skipped
            let event = result.ok()?;
            let mut out = Vec::with_capacity(2);
            if let Ok(data) = serde_json::to_string(&event) {
                out.push(Ok(Event::default().event(event.event_type()).data(data)));
            }
            match turbo_streams_for(&state, &event).await {
                Ok(html) if !html.is_empty() => out.push(Ok(Event::default().data(html))),
                Ok(_) => {}
                Err(e) => tracing::warn!("Failed to render turbo streams: {:?}", e),
            }
            Some(futures::stream::iter(out))
        }
    });

    Sse::new(stream.flatten()).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keep-alive"),
    )
}
