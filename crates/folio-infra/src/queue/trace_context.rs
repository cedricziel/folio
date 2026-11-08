//! W3C trace context stored with a job, so its span continues the trace that queued it.

use std::collections::HashMap;

use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::TraceContextExt;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// The `traceparent` and `tracestate` headers of the span a job was queued under
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobTraceContext {
    pub traceparent: String,
    pub tracestate: Option<String>,
}

impl JobTraceContext {
    /// The context of the current span, or `None` outside a trace
    pub(crate) fn current() -> Option<Self> {
        let context = tracing::Span::current().context();
        if !context.span().span_context().is_valid() {
            return None;
        }
        let mut headers = HashMap::new();
        TraceContextPropagator::new().inject_context(&context, &mut headers);
        Some(Self {
            traceparent: headers.remove("traceparent")?,
            tracestate: headers
                .remove("tracestate")
                .filter(|state| !state.is_empty()),
        })
    }

    /// Stored columns as a job context; `None` unless `traceparent` is present
    pub(crate) fn from_columns(
        traceparent: Option<String>,
        tracestate: Option<String>,
    ) -> Option<Self> {
        Some(Self {
            traceparent: traceparent?,
            tracestate,
        })
    }

    /// The remote parent for the job span; empty when the stored value does not parse
    pub(crate) fn to_parent(&self) -> opentelemetry::Context {
        let mut headers = HashMap::from([("traceparent".to_string(), self.traceparent.clone())]);
        if let Some(state) = &self.tracestate {
            headers.insert("tracestate".to_string(), state.clone());
        }
        TraceContextPropagator::new().extract(&headers)
    }
}
