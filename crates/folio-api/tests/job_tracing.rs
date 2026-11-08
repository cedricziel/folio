//! A job queued by a request continues that request's trace.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use opentelemetry::trace::{TraceContextExt, TraceId, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tower::ServiceExt;
use tracing::dispatcher::WeakDispatch;
use tracing::span;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use folio_infra::{Worker, WorkerConfig};

const TRACE_ID: &str = "11223344556677881122334455667788";

/// Trace ID of each span by name, read when the span is exited
///
/// Exported spans are not enough: some request and job spans stay open after
/// their work is done, so they may never reach an exporter during a test.
#[derive(Clone, Default)]
struct TraceIds {
    seen: Arc<Mutex<Vec<(String, TraceId)>>>,
    dispatch: Arc<Mutex<Option<WeakDispatch>>>,
}

impl TraceIds {
    fn of(&self, name: &str) -> Vec<TraceId> {
        let seen = self.seen.lock().unwrap();
        let mut ids: Vec<_> = seen
            .iter()
            .filter(|(seen, _)| seen == name)
            .map(|(_, id)| *id)
            .collect();
        ids.dedup();
        ids
    }
}

impl<S: tracing::Subscriber + for<'a> LookupSpan<'a>> Layer<S> for TraceIds {
    fn on_register_dispatch(&self, subscriber: &tracing::Dispatch) {
        *self.dispatch.lock().unwrap() = Some(subscriber.downgrade());
    }

    fn on_exit(&self, id: &span::Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let name = span.metadata().name();
        if name != "job" && name != "HTTP request" {
            return;
        }
        let Some(dispatch) = self
            .dispatch
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|d| d.upgrade())
        else {
            return;
        };
        if let Some(cx) = tracing_opentelemetry::get_otel_context(id, &dispatch) {
            let trace_id = cx.span().span_context().trace_id();
            self.seen.lock().unwrap().push((name.to_string(), trace_id));
        }
    }
}

fn tracing_setup() -> (TraceIds, tracing::subscriber::DefaultGuard) {
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(InMemorySpanExporter::default())
        .build();
    let trace_ids = TraceIds::default();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("folio-test")))
        .with(trace_ids.clone());
    (trace_ids, tracing::subscriber::set_default(subscriber))
}

fn worker(h: &common::Harness) -> Worker {
    let state = &h.state;
    Worker::new(
        state.job_queue.clone(),
        state.document_repo.clone(),
        state.ocr_service.clone(),
        state.embedding_service.clone(),
        state.search_service.clone(),
        state.pipeline.clone(),
        state.thumbnail_service.clone(),
        state.store.clone(),
        WorkerConfig {
            poll_interval_ms: 10,
            ..WorkerConfig::default()
        },
        state.outbox.clone(),
    )
}

async fn upload(h: &common::Harness, traceparent: Option<String>) {
    let mut request = common::multipart(&[("file", Some("scan.png"), "not really a png")]);
    *request.uri_mut() = "/api/documents/upload".parse().unwrap();
    if let Some(traceparent) = traceparent {
        request
            .headers_mut()
            .insert("traceparent", traceparent.parse().unwrap());
    }
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert!(response.status().is_success(), "{}", response.status());
}

/// Run the worker until the queued `process_document` job has finished; return its status and error.
async fn run_process_document(h: &common::Harness) -> (String, Option<String>) {
    let folio_infra::Database::Sqlite(pool) = &h.database else {
        unreachable!("the harness uses SQLite");
    };
    let worker = worker(h);
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let finished = async {
        loop {
            let (status, error): (String, Option<String>) = sqlx::query_as(
                "SELECT status, error FROM jobs WHERE job_type = 'process_document'",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            if status == "completed" || status == "failed" {
                stop.send(true).unwrap();
                return (status, error);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let (_, outcome) = tokio::time::timeout(
        Duration::from_secs(30),
        futures::future::join(worker.run(shutdown), finished),
    )
    .await
    .expect("the worker finishes the queued document");
    outcome
}

#[tokio::test]
async fn an_uploads_processing_job_continues_the_request_trace() {
    let (trace_ids, _default) = tracing_setup();
    let h = common::harness().await;
    upload(&h, Some(format!("00-{TRACE_ID}-1234567812345678-01"))).await;

    run_process_document(&h).await;

    let expected = TraceId::from_hex(TRACE_ID).unwrap();
    assert_eq!(trace_ids.of("HTTP request"), vec![expected]);
    assert_eq!(trace_ids.of("job"), vec![expected]);
}

#[tokio::test]
async fn a_job_queued_without_trace_context_still_runs_in_a_new_trace() {
    let (trace_ids, _default) = tracing_setup();
    let h = common::harness().await;
    upload(&h, None).await;
    let folio_infra::Database::Sqlite(pool) = &h.database else {
        unreachable!("the harness uses SQLite");
    };
    // As if queued before trace context was stored, or stored in a form that does not parse.
    sqlx::query(
        "UPDATE jobs SET trace_parent = 'not-a-traceparent', trace_state = NULL \
         WHERE job_type = 'process_document'",
    )
    .execute(pool)
    .await
    .unwrap();

    let (status, error) = run_process_document(&h).await;

    assert_eq!((status.as_str(), error), ("completed", None));
    let request = trace_ids.of("HTTP request");
    let job = trace_ids.of("job");
    assert_eq!(request.len(), 1);
    assert_eq!(job.len(), 1, "{job:?}");
    assert_ne!(job[0], TraceId::INVALID);
    assert_ne!(job[0], request[0], "the job starts a new trace");
}

#[tokio::test]
async fn re_embedding_every_document_queues_untraced_jobs() {
    let (_trace_ids, _default) = tracing_setup();
    let h = common::harness().await;
    for title in ["First", "Second"] {
        h.state
            .document_repo
            .create(
                h.vault,
                folio_core::models::CreateDocument {
                    title: title.into(),
                    content: Some("text".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/settings/processing/reembed")
        .header("host", "localhost")
        .header("origin", "http://localhost")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("traceparent", format!("00-{TRACE_ID}-1234567812345678-01"))
        .body(axum::body::Body::empty())
        .unwrap();
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.headers()["location"], "/settings?reembedded=2");

    let folio_infra::Database::Sqlite(pool) = &h.database else {
        unreachable!("the harness uses SQLite");
    };
    let traced: Vec<Option<String>> =
        sqlx::query_scalar("SELECT trace_parent FROM jobs WHERE job_type = 'generate_embedding'")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(
        traced,
        [None, None],
        "one request must not put every job in its trace"
    );
}
