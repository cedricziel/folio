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
use crate::events::DocumentEvent;
use crate::state::AppState;

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

/// SSE endpoint for real-time document updates.
///
/// Each event is sent twice: as the typed JSON event and as an unnamed
/// message carrying Turbo Streams, which Turbo applies to the page as is.
async fn sse_handler(
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
