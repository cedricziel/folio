use std::future::Future;
use std::io;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, Method};
use axum::middleware::Next;
use axum::response::Response;
use futures::StreamExt;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

const HTTP_DRAIN_LIMIT: Duration = Duration::from_secs(10);
const BACKGROUND_DRAIN_LIMIT: Duration = Duration::from_secs(10);

pub(crate) async fn close_event_streams(
    State(shutdown): State<CancellationToken>,
    request: Request,
    next: Next,
) -> Response {
    let subscription = request.method() == Method::GET;
    let response = next.run(request).await;
    if subscription
        && response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|v| {
                v.as_bytes()
                    .split(|&byte| byte == b';')
                    .next()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case(b"text/event-stream"))
            })
    {
        let (parts, body) = response.into_parts();
        let stream = body
            .into_data_stream()
            .take_until(shutdown.cancelled_owned());
        Response::from_parts(parts, Body::from_stream(stream))
    } else {
        response
    }
}

pub(crate) async fn drain_http(
    server: impl Future<Output = io::Result<()>>,
) -> Option<io::Result<()>> {
    match timeout(HTTP_DRAIN_LIMIT, server).await {
        Ok(result) => Some(result),
        Err(_) => {
            tracing::warn!("HTTP shutdown grace period expired");
            None
        }
    }
}

pub(crate) async fn wait_for_background(mut tasks: JoinSet<()>) {
    let drained = timeout(BACKGROUND_DRAIN_LIMIT, async {
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                tracing::warn!(%error, "Background task failed");
            }
        }
    })
    .await;
    if drained.is_err() {
        tracing::warn!("Background shutdown grace period expired");
        tasks.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn http_drain_gives_up_when_a_request_never_finishes() {
        let started = tokio::time::Instant::now();

        assert!(drain_http(std::future::pending()).await.is_none());

        assert_eq!(started.elapsed(), HTTP_DRAIN_LIMIT);
    }

    #[tokio::test(start_paused = true)]
    async fn background_drain_allows_bounded_work_and_aborts_stalled_tasks() {
        let mut tasks = JoinSet::new();
        let (finished, completion) = tokio::sync::oneshot::channel();
        tasks.spawn(async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            finished.send(()).unwrap();
        });
        let stalled = tasks.spawn(std::future::pending());
        let started = tokio::time::Instant::now();

        wait_for_background(tasks).await;

        assert_eq!(started.elapsed(), BACKGROUND_DRAIN_LIMIT);
        assert!(completion.await.is_ok());
        assert!(stalled.is_finished());
    }
}
