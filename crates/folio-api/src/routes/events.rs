//! Server-Sent Events routes

use axum::{
    extract::State,
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use futures::stream::{self, Stream, StreamExt as _};
use std::collections::{HashSet, VecDeque};
use std::{convert::Infallible, time::Duration};
use tokio::sync::broadcast;

use crate::error::ApiResult;
use crate::state::AppState;
use folio_core::events::{DocumentEvent, OutboxEvent};

use super::pages::DocumentStreams;

pub fn router() -> Router<AppState> {
    Router::new().route("/events", get(sse_handler))
}

/// Turbo Streams that bring open pages up to date after `event`.
pub async fn turbo_streams_for(state: &AppState, event: &DocumentEvent) -> ApiResult<String> {
    let streams = match event {
        DocumentEvent::Deleted { document_id, .. } => {
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

/// How far back a reconnecting client is caught up
const REPLAY_WINDOW: chrono::Duration = chrono::Duration::hours(1);
/// Most events replayed to one reconnecting client
const REPLAY_LIMIT: u32 = 500;

/// Subscribe to real-time document events
///
/// Each SSE message has the event name from `DocumentEvent` (for example `created`)
/// and a JSON `DocumentEvent` as data. Each event is followed by an unnamed message holding the
/// Turbo Streams that update the web UI. Comment-only keep-alive frames are sent every 30 seconds.
///
/// Message ids are the ids of the persisted events. A client that reconnects
/// with `Last-Event-ID` first receives the events it missed (at most 500 from
/// the last hour), then the live stream.
#[utoipa::path(
    get,
    path = "/events",
    operation_id = "streamEvents",
    tag = "events",
    params(
        ("Last-Event-ID" = Option<String>, Header, description = "Id of the last event received; events after it are replayed first"),
    ),
    responses(
        (status = 200, description = "Stream of document events", body = DocumentEvent, content_type = "text/event-stream"),
    )
)]
pub(crate) async fn sse_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok());

    // Subscribe before replaying, so nothing falls into the gap between the two.
    let mut connection = Connection {
        rx: state.event_broadcaster.subscribe(),
        state,
        pending: VecDeque::new(),
        last_id: last_event_id,
        connected_at: Utc::now(),
        seen: HashSet::new(),
        seen_order: VecDeque::new(),
    };
    if let Some(after) = last_event_id {
        connection.replay(after, Utc::now() - REPLAY_WINDOW).await;
    }

    let stream = stream::unfold(connection, |mut conn| async move {
        let event = conn.next_event().await?;
        let frames = frames_for(&conn.state, &event).await;
        Some((stream::iter(frames.into_iter().map(Ok)), conn))
    });

    Sse::new(stream.flatten()).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keep-alive"),
    )
}

/// The SSE messages for one event: the JSON event, then its Turbo Streams
async fn frames_for(state: &AppState, outbox_event: &OutboxEvent) -> Vec<Event> {
    let id = outbox_event.id.to_string();
    let event = &outbox_event.event;
    let mut frames = Vec::with_capacity(2);
    if let Ok(data) = serde_json::to_string(event) {
        frames.push(
            Event::default()
                .id(id.clone())
                .event(event.event_type())
                .data(data),
        );
    }
    match turbo_streams_for(state, event).await {
        Ok(html) if !html.is_empty() => frames.push(Event::default().id(id).data(html)),
        Ok(_) => {}
        Err(e) => tracing::warn!("Failed to render turbo streams: {:?}", e),
    }
    frames
}

/// One open SSE connection
struct Connection {
    state: AppState,
    rx: broadcast::Receiver<OutboxEvent>,
    /// Replayed events not yet sent
    pending: VecDeque<OutboxEvent>,
    /// Newest event sent or acknowledged by the client
    last_id: Option<i64>,
    connected_at: DateTime<Utc>,
    seen: HashSet<i64>,
    seen_order: VecDeque<i64>,
}

impl Connection {
    fn remember(&mut self, id: i64) -> bool {
        if !self.seen.insert(id) {
            return false;
        }
        self.last_id = Some(id);
        self.seen_order.push_back(id);
        if self.seen_order.len() > REPLAY_LIMIT as usize {
            if let Some(oldest) = self.seen_order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        true
    }

    async fn replay(&mut self, after: i64, since: DateTime<Utc>) {
        match self.state.outbox.replay(after, since, REPLAY_LIMIT).await {
            Ok(events) => {
                for event in events {
                    if !self.seen.contains(&event.id) {
                        self.pending.push_back(event);
                    }
                }
            }
            Err(e) => tracing::warn!("Failed to replay events: {e}"),
        }
    }

    /// The next event to send, or `None` once the server shuts down
    async fn next_event(&mut self) -> Option<OutboxEvent> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                if self.remember(event.id) {
                    return Some(event);
                }
                continue;
            }
            match self.rx.recv().await {
                Ok(event) => {
                    if self.remember(event.id) {
                        return Some(event);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::debug!("SSE client missed {missed} events, replaying");
                    match self.last_id {
                        Some(after) => self.replay(after, Utc::now() - REPLAY_WINDOW).await,
                        None => self.replay(0, self.connected_at).await,
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
