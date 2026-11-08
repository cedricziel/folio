//! A job stores the W3C trace context it was queued under.

mod common;

use folio_core::models::CreateDocument;
use folio_infra::{Database, JobType};
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use std::collections::HashMap;
use tracing::instrument::WithSubscriber;
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;

const TRACE_ID: &str = "11223344556677881122334455667788";

async fn contract(db: &Database) {
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(InMemorySpanExporter::default())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("folio-test")));
    let _default = tracing::subscriber::set_default(subscriber);

    let repos = db.repositories().await;
    let vault = repos.vaults.create("Traced", None).await.unwrap().id;
    let document = repos
        .documents
        .create(
            vault,
            CreateDocument {
                title: "Traced document".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let headers = HashMap::from([
        (
            "traceparent".to_string(),
            format!("00-{TRACE_ID}-1234567812345678-01"),
        ),
        ("tracestate".to_string(), "folio=upload".to_string()),
    ]);
    let request = tracing::info_span!("request");
    let _ = request.set_parent(TraceContextPropagator::new().extract(&headers));
    let id = repos
        .jobs
        .enqueue(JobType::process_document(document.id))
        .instrument(request)
        .await
        .unwrap();

    let stored = repos.jobs.trace_context(id).await.unwrap().unwrap();
    assert!(
        stored.traceparent.starts_with(&format!("00-{TRACE_ID}-")),
        "{}",
        stored.traceparent
    );
    assert!(
        stored.traceparent.ends_with("-01"),
        "{}",
        stored.traceparent
    );
    assert_eq!(stored.tracestate.as_deref(), Some("folio=upload"));

    let untraced = repos
        .jobs
        .enqueue(JobType::generate_embedding(document.id))
        .with_subscriber(tracing::subscriber::NoSubscriber::default())
        .await
        .unwrap();
    assert_eq!(
        repos.jobs.trace_context(untraced).await.unwrap(),
        None,
        "a job queued with telemetry off stores no context"
    );
    assert_eq!(
        repos.jobs.trace_context(Uuid::new_v4()).await.unwrap(),
        None
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
