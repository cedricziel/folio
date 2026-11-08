//! HTTP server telemetry: a span per request and request metrics, named after
//! the OpenTelemetry HTTP semantic conventions.

use std::time::{Duration, Instant};

use axum::http::{header, HeaderValue};
use axum::{
    extract::{MatchedPath, Request, State},
    middleware::{self, Next},
    response::Response,
    Router,
};
use opentelemetry::metrics::{Histogram, UpDownCounter};
use opentelemetry::trace::TraceContextExt;
use opentelemetry::{global, Array, KeyValue, StringValue, Value};
use opentelemetry_http::HeaderExtractor;
use tower_http::trace::TraceLayer;
use tracing::{field, info_span, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Latency buckets recommended by the HTTP semantic conventions, in seconds.
const DURATION_BUCKETS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

#[derive(Clone)]
struct HttpMetrics {
    duration: Histogram<f64>,
    active: UpDownCounter<i64>,
}

impl HttpMetrics {
    fn new() -> Self {
        let meter = global::meter("folio.http");
        Self {
            duration: meter
                .f64_histogram("http.server.request.duration")
                .with_unit("s")
                .with_description("Duration of HTTP server requests")
                .with_boundaries(DURATION_BUCKETS.to_vec())
                .build(),
            active: meter
                .i64_up_down_counter("http.server.active_requests")
                .with_unit("{request}")
                .with_description("HTTP server requests in flight")
                .build(),
        }
    }
}

/// Wrap `router` with request tracing and metrics. Apply it after all routes
/// are added so the matched route is known when the span opens.
pub fn instrument(router: Router<crate::AppState>) -> Router<crate::AppState> {
    router
        .layer(middleware::from_fn(server_timing))
        .layer(middleware::from_fn_with_state(
            HttpMetrics::new(),
            record_metrics,
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(make_span)
                .on_response(|response: &Response, _latency: Duration, span: &Span| {
                    let status = response.status();
                    span.record("http.response.status_code", i64::from(status.as_u16()));
                    if status.is_server_error() {
                        span.record("otel.status_code", "ERROR");
                    }
                })
                .on_failure(
                    |error: tower_http::classify::ServerErrorsFailureClass,
                     _latency: Duration,
                     span: &Span| {
                        span.record("otel.status_code", "ERROR");
                        span.record("error.type", error.to_string());
                    },
                ),
        )
}

fn make_span(request: &Request) -> Span {
    if request.uri().path().starts_with("/api/telemetry/") {
        return Span::none();
    }
    let method = request.method().as_str();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());
    let name = match &route {
        Some(route) => format!("{method} {route}"),
        None => method.to_owned(),
    };
    let user_agent = request
        .headers()
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();

    let span = info_span!(
        "HTTP request",
        otel.name = %name,
        otel.kind = "server",
        otel.status_code = field::Empty,
        http.request.method = method,
        http.route = field::Empty,
        http.response.status_code = field::Empty,
        url.path = request.uri().path(),
        user_agent.original = user_agent,
        error.type = field::Empty,
    );
    if let Some(route) = &route {
        span.record("http.route", route.as_str());
    }

    // Continue a trace started by the caller (W3C traceparent).
    let parent = global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(request.headers()))
    });
    let _ = span.set_parent(parent);
    if let Some((scope, filters)) = route
        .as_deref()
        .and_then(|route| browse_context(route, request.uri().query()))
    {
        span.set_attribute("folio.browse.scope", scope);
        let filters = filters.into_iter().map(StringValue::from).collect();
        span.set_attribute("folio.browse.filters", Value::Array(Array::String(filters)));
    }
    span
}

/// Which document list a page request shows and which kinds of filter it uses.
/// Only fixed names are returned, never the filter values.
fn browse_context(route: &str, query: Option<&str>) -> Option<(&'static str, Vec<&'static str>)> {
    let pairs = || {
        url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
            .filter(|(_, value)| !value.is_empty())
    };
    match route {
        "/" => {
            let mut filters: Vec<&'static str> = pairs()
                .filter_map(|(key, _)| match key.as_ref() {
                    "reviewed" => Some("reviewed"),
                    "document_date_from" | "document_date_to" => Some("document_date"),
                    "correspondent_id" => Some("correspondent"),
                    "document_type_id" => Some("document_type"),
                    "tag_id" => Some("tag"),
                    _ => None,
                })
                .collect();
            filters.sort_unstable();
            filters.dedup();
            // Mirrors the index page: a taxonomy filter or view=all lists every document.
            let all = pairs().any(|(key, value)| key == "view" && value == "all")
                || filters
                    .iter()
                    .any(|kind| matches!(*kind, "correspondent" | "document_type" | "tag"));
            Some((if all { "all" } else { "inbox" }, filters))
        }
        "/documents" => Some(("all", Vec::new())),
        "/documents/search" => {
            let text = pairs().any(|(key, _)| key == "q");
            Some(("search", if text { vec!["text"] } else { Vec::new() }))
        }
        _ => None,
    }
}

/// Names the server request span on HTML responses, so a full page load's browser
/// navigation span can join the same trace. Holds only trace and span IDs.
async fn server_timing(request: Request, next: Next) -> Response {
    if request.uri().path().starts_with("/api/") {
        return next.run(request).await;
    }
    let context = Span::current().context();
    let span = context.span();
    let span_context = span.span_context().clone();
    let mut response = next.run(request).await;
    let html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));
    if html && span_context.is_valid() {
        let value = format!(
            "traceparent;desc=\"00-{}-{}-{:02x}\"",
            span_context.trace_id(),
            span_context.span_id(),
            span_context.trace_flags().to_u8()
        );
        if let Ok(value) = HeaderValue::from_str(&value) {
            response
                .headers_mut()
                .append(header::HeaderName::from_static("server-timing"), value);
        }
    }
    response
}

async fn record_metrics(
    State(metrics): State<HttpMetrics>,
    request: Request,
    next: Next,
) -> Response {
    if request.uri().path().starts_with("/api/telemetry/") {
        return next.run(request).await;
    }
    let method = request.method().as_str().to_owned();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());

    metrics
        .active
        .add(1, &[KeyValue::new("http.request.method", method.clone())]);
    let started = Instant::now();
    let response = next.run(request).await;
    metrics
        .active
        .add(-1, &[KeyValue::new("http.request.method", method.clone())]);

    let mut attributes = vec![
        KeyValue::new("http.request.method", method),
        KeyValue::new(
            "http.response.status_code",
            i64::from(response.status().as_u16()),
        ),
    ];
    if let Some(route) = route {
        attributes.push(KeyValue::new("http.route", route));
    }
    metrics
        .duration
        .record(started.elapsed().as_secs_f64(), &attributes);
    response
}
