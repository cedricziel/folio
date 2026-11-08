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
    global::set_text_map_propagator(opentelemetry_sdk::propagation::TraceContextPropagator::new());
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
                .header(
                    "traceparent",
                    "00-11223344556677881122334455667788-1234567812345678-01",
                )
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
        server.span_context.trace_id().to_string(),
        "11223344556677881122334455667788"
    );
    assert_eq!(server.parent_span_id.to_string(), "1234567812345678");
    assert_eq!(
        attribute(server.attributes.iter(), "http.route").as_deref(),
        Some("/api/documents/{id}")
    );
    assert_eq!(
        attribute(server.attributes.iter(), "http.request.method").as_deref(),
        Some("GET")
    );
    // Semantic conventions type the status code as an int; a string is not filterable as one.
    assert_eq!(
        server
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "http.response.status_code")
            .map(|kv| kv.value.clone()),
        Some(opentelemetry::Value::I64(404))
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

fn install_tracer() -> (InMemorySpanExporter, tracing::subscriber::DefaultGuard) {
    global::set_text_map_propagator(opentelemetry_sdk::propagation::TraceContextPropagator::new());
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("folio-test")));
    (exporter, tracing::subscriber::set_default(subscriber))
}

async fn server_span(
    app: &axum::Router,
    exporter: &InMemorySpanExporter,
    uri: &str,
) -> (opentelemetry_sdk::trace::SpanData, axum::http::HeaderMap) {
    exporter.reset();
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{uri}: {}",
        response.status()
    );
    let headers = response.headers().clone();
    drop(response);
    let span = exporter
        .get_finished_spans()
        .unwrap()
        .into_iter()
        .find(|span| span.span_kind == SpanKind::Server)
        .expect("a server span");
    (span, headers)
}

fn value(span: &opentelemetry_sdk::trace::SpanData, key: &str) -> Option<opentelemetry::Value> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.clone())
}

fn filters(span: &opentelemetry_sdk::trace::SpanData) -> Option<Vec<String>> {
    match value(span, "folio.browse.filters")? {
        opentelemetry::Value::Array(opentelemetry::Array::String(values)) => {
            Some(values.into_iter().map(|v| v.to_string()).collect())
        }
        other => panic!("filters must be a string array, got {other:?}"),
    }
}

#[tokio::test]
async fn document_list_spans_record_browse_scope_and_filter_kinds_only() {
    let (exporter, _guard) = install_tracer();
    let harness = common::harness().await;
    let tag = Uuid::new_v4();

    let (inbox, _) = server_span(&harness.app, &exporter, "/").await;
    assert_eq!(
        attribute(inbox.attributes.iter(), "folio.browse.scope").as_deref(),
        Some("inbox")
    );
    assert_eq!(filters(&inbox), Some(vec![]));

    let uri = format!("/?view=all&tag_id={tag}&reviewed=false&document_date_from=2026-01-01");
    let (all, _) = server_span(&harness.app, &exporter, &uri).await;
    assert_eq!(
        attribute(all.attributes.iter(), "folio.browse.scope").as_deref(),
        Some("all")
    );
    assert_eq!(
        filters(&all),
        Some(vec![
            "document_date".into(),
            "reviewed".into(),
            "tag".into()
        ])
    );

    let (tagged, _) = server_span(&harness.app, &exporter, &format!("/?tag_id={tag}")).await;
    assert_eq!(
        attribute(tagged.attributes.iter(), "folio.browse.scope").as_deref(),
        Some("all")
    );

    // The index page ignores a tag filter that isn't an ID, so it isn't counted either.
    let (malformed, _) = server_span(&harness.app, &exporter, "/?tag_id=not-a-tag").await;
    assert_eq!(
        attribute(malformed.attributes.iter(), "folio.browse.scope").as_deref(),
        Some("inbox")
    );
    assert_eq!(filters(&malformed), Some(vec![]));

    let (palette, _) = server_span(
        &harness.app,
        &exporter,
        "/api/documents/search?q=private+words",
    )
    .await;
    assert_eq!(
        attribute(palette.attributes.iter(), "folio.browse.scope").as_deref(),
        Some("search")
    );
    assert_eq!(filters(&palette), Some(vec!["text".into()]));

    let (search, _) =
        server_span(&harness.app, &exporter, "/documents/search?q=private+words").await;
    assert_eq!(
        attribute(search.attributes.iter(), "folio.browse.scope").as_deref(),
        Some("search")
    );
    assert_eq!(filters(&search), Some(vec!["text".into()]));

    for span in [&inbox, &all, &tagged, &malformed, &palette, &search] {
        for kv in span.attributes.iter() {
            let text = kv.value.to_string();
            assert!(!text.contains(&tag.to_string()), "{kv:?}");
            assert!(!text.contains("private"), "{kv:?}");
            assert!(!text.contains("2026-01-01"), "{kv:?}");
        }
    }
}

#[tokio::test]
async fn html_responses_name_their_server_trace_for_the_browser() {
    let (exporter, _guard) = install_tracer();
    let harness = common::harness().await;

    let (span, headers) = server_span(&harness.app, &exporter, "/").await;
    let expected = format!(
        "traceparent;desc=\"00-{}-{}-01\"",
        span.span_context.trace_id(),
        span.span_context.span_id()
    );
    assert_eq!(
        headers.get("server-timing").and_then(|v| v.to_str().ok()),
        Some(expected.as_str())
    );

    let (_, json) = server_span(&harness.app, &exporter, "/api/tags").await;
    assert!(json.get("server-timing").is_none());

    exporter.reset();
    let missing = harness
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/no-such-page")
                .header("accept", "text/html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
    let timing = missing
        .headers()
        .get("server-timing")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    drop(missing);
    let span = exporter
        .get_finished_spans()
        .unwrap()
        .into_iter()
        .find(|span| span.span_kind == SpanKind::Server)
        .expect("a server span for the error page");
    assert_eq!(
        timing.as_deref(),
        Some(
            format!(
                "traceparent;desc=\"00-{}-{}-01\"",
                span.span_context.trace_id(),
                span.span_context.span_id()
            )
            .as_str()
        ),
        "HTML error pages name their trace too"
    );
}
