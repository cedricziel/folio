//! A request produces a server span and request metrics.

mod common;

use axum::body::Body;
use axum::http::Request;
use opentelemetry::global;
use opentelemetry::trace::{SpanKind, TracerProvider as _};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use tower::ServiceExt;
use tracing_subscriber::layer::SubscriberExt;
use uuid::Uuid;

fn attribute<'a>(
    attributes: impl Iterator<Item = &'a opentelemetry::KeyValue>,
    key: &str,
) -> Option<String> {
    attributes
        .into_iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.to_string())
}

#[tokio::test]
async fn a_request_produces_a_server_span_and_duration_metric() {
    let span_exporter = InMemorySpanExporter::default();
    let tracer_provider = SdkTracerProvider::builder()
        .with_simple_exporter(span_exporter.clone())
        .build();
    let metric_exporter = InMemoryMetricExporter::default();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(metric_exporter.clone()).build())
        .build();
    global::set_meter_provider(meter_provider.clone());

    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(tracer_provider.tracer("folio-test")));
    let _default = tracing::subscriber::set_default(subscriber);

    // The harness builds the router after the meter provider is installed.
    let harness = common::harness().await;
    let id = Uuid::new_v4();
    let response = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/documents/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    // The span ends when the response body is dropped.
    drop(response);

    let spans = span_exporter.get_finished_spans().unwrap();
    let server = spans
        .iter()
        .find(|span| span.span_kind == SpanKind::Server)
        .expect("a server span");
    assert_eq!(server.name, "GET /api/documents/{id}");
    assert_eq!(
        attribute(server.attributes.iter(), "http.route").as_deref(),
        Some("/api/documents/{id}")
    );
    assert_eq!(
        attribute(server.attributes.iter(), "http.request.method").as_deref(),
        Some("GET")
    );
    assert_eq!(
        attribute(server.attributes.iter(), "http.response.status_code").as_deref(),
        Some("404")
    );

    meter_provider.force_flush().unwrap();
    let metrics = metric_exporter.get_finished_metrics().unwrap();
    let names: Vec<_> = metrics
        .iter()
        .flat_map(|rm| rm.scope_metrics())
        .flat_map(|sm| sm.metrics())
        .map(|m| m.name().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n == "http.server.request.duration"),
        "{names:?}"
    );
}
