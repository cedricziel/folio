//! A job queued by a request continues that request's trace.

mod common;

use std::time::Duration;

use opentelemetry::trace::{SpanKind, TraceId, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use tower::ServiceExt;
use tracing_subscriber::layer::SubscriberExt;

use folio_infra::{Worker, WorkerConfig};

const TRACE_ID: &str = "11223344556677881122334455667788";

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

/// Run the worker until the queued `process_document` job has finished; return its span,
/// status and error.
async fn run_process_document(
    h: &common::Harness,
    exporter: &InMemorySpanExporter,
) -> (SpanData, String, Option<String>) {
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
    let (_, (status, error)) = tokio::time::timeout(
        Duration::from_secs(30),
        futures::future::join(worker.run(shutdown), finished),
    )
    .await
    .expect("the worker finishes the queued document");
    // SQLite connection threads keep the span of their last command alive, so
    // the job span may only end once the pool closes.
    pool.close().await;
    let span = exporter
        .get_finished_spans()
        .unwrap()
        .into_iter()
        .find(|span| span.name == "job process_document")
        .expect("a job process_document span");
    (span, status, error)
}

fn tracing_setup() -> (InMemorySpanExporter, tracing::subscriber::DefaultGuard) {
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("folio-test")));
    (exporter, tracing::subscriber::set_default(subscriber))
}

#[tokio::test]
async fn an_uploads_processing_job_continues_the_request_trace() {
    let (exporter, _default) = tracing_setup();
    let h = common::harness().await;
    let mut request = common::multipart(&[("file", Some("scan.png"), "not really a png")]);
    *request.uri_mut() = "/api/documents/upload".parse().unwrap();
    request.headers_mut().insert(
        "traceparent",
        format!("00-{TRACE_ID}-1234567812345678-01")
            .parse()
            .unwrap(),
    );
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    drop(response);

    let (job, _, _) = run_process_document(&h, &exporter).await;

    assert_eq!(job.span_kind, SpanKind::Consumer);
    assert_eq!(job.span_context.trace_id().to_string(), TRACE_ID);
    let spans = exporter.get_finished_spans().unwrap();
    assert!(
        spans
            .iter()
            .any(|span| span.span_context.span_id() == job.parent_span_id),
        "the job's parent is a span of the request"
    );
}

#[tokio::test]
async fn a_job_queued_without_trace_context_still_runs_in_a_new_trace() {
    let (exporter, _default) = tracing_setup();
    let h = common::harness().await;
    let mut request = common::multipart(&[("file", Some("scan.png"), "not really a png")]);
    *request.uri_mut() = "/api/documents/upload".parse().unwrap();
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    drop(response);
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

    let (job, status, error) = run_process_document(&h, &exporter).await;

    assert_ne!(job.span_context.trace_id(), TraceId::INVALID);
    assert_eq!(job.parent_span_id, opentelemetry::trace::SpanId::INVALID);
    assert_eq!((status.as_str(), error), ("completed", None));
}
