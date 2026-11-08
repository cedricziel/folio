//! A job stores the W3C trace context of the span that queued it.

mod common;

use folio_core::models::CreateDocument;
use folio_infra::{Database, JobTraceContext, JobType};
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::{TraceContextExt, TracerProvider as _};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use std::collections::HashMap;
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;

const TRACE_ID: &str = "11223344556677881122334455667788";

async fn document(db: &Database, vault: folio_core::models::VaultId) -> Uuid {
    db.repositories()
        .await
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Traced document".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
}

/// Claim the only pending job and return the trace context it carries.
async fn claimed_trace(db: &Database, id: Uuid) -> Option<JobTraceContext> {
    let job = db
        .repositories()
        .await
        .jobs
        .dequeue()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.id, id);
    job.trace
}

fn request_span() -> tracing::Span {
    let headers = HashMap::from([
        (
            "traceparent".to_string(),
            format!("00-{TRACE_ID}-1234567812345678-01"),
        ),
        ("tracestate".to_string(), "folio=upload".to_string()),
    ]);
    let request = tracing::info_span!("request");
    let _ = request.set_parent(TraceContextPropagator::new().extract(&headers));
    request
}

async fn contract(db: &Database) {
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(InMemorySpanExporter::default())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("folio-test")));
    let _default = tracing::subscriber::set_default(subscriber);
    let repos = db.repositories().await;
    let vault = repos.vaults.create("Traced", None).await.unwrap().id;

    let request = request_span();
    let caller = request.context().span().span_context().span_id();
    let id = repos
        .jobs
        .enqueue(JobType::process_document(document(db, vault).await))
        .instrument(request)
        .await
        .unwrap();
    let stored = claimed_trace(db, id).await.unwrap();
    assert_eq!(
        stored.traceparent,
        format!("00-{TRACE_ID}-{caller}-01"),
        "the caller's span, not the queue's own, is the parent"
    );
    assert_eq!(stored.tracestate.as_deref(), Some("folio=upload"));

    let background = repos
        .jobs
        .enqueue(JobType::process_document(document(db, vault).await))
        .await
        .unwrap();
    assert_eq!(
        claimed_trace(db, background).await,
        None,
        "a job queued outside any span stores no context"
    );

    let bulk = repos
        .jobs
        .enqueue_untraced(JobType::process_document(document(db, vault).await))
        .instrument(request_span())
        .await
        .unwrap();
    assert_eq!(
        claimed_trace(db, bulk).await,
        None,
        "a bulk enqueue stores no context"
    );
}

#[tokio::test]
async fn sqlite_job_trace_context_round_trip() {
    let (_dir, db) = common::sqlite_database().await;
    contract(&db).await;
}

#[tokio::test]
async fn postgres_job_trace_context_round_trip() {
    let Some((_container, db)) = common::postgres_database("PostgreSQL").await else {
        return;
    };
    contract(&db).await;
}
