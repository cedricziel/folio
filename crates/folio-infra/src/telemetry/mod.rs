//! OpenTelemetry setup
//!
//! Export is configured only through the standard `OTEL_*` environment
//! variables. Without an OTLP endpoint (or with `OTEL_SDK_DISABLED=true`)
//! nothing is exported and logs go to stdout as before.
//!
//! Honored variables: `OTEL_EXPORTER_OTLP_ENDPOINT` (and the per-signal
//! `_TRACES_`, `_METRICS_`, `_LOGS_` forms), `OTEL_EXPORTER_OTLP_PROTOCOL`
//! (`http/protobuf` default, or `grpc`), `OTEL_EXPORTER_OTLP_HEADERS`,
//! `OTEL_SERVICE_NAME` (default `folio`), `OTEL_RESOURCE_ATTRIBUTES`,
//! `OTEL_SDK_DISABLED`, and `OTEL_{TRACES,METRICS,LOGS}_EXPORTER=none`.

mod metrics;

use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, MetricExporter, SpanExporter};
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use tracing::warn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry};

pub use metrics::{metrics, Metrics};

const DEFAULT_SERVICE_NAME: &str = "folio";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtlpProtocol {
    HttpProtobuf,
    Grpc,
}

/// Which signals to export and how, derived from `OTEL_*` variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetrySettings {
    pub protocol: OtlpProtocol,
    pub traces: bool,
    pub metrics: bool,
    pub logs: bool,
}

impl TelemetrySettings {
    /// `None` when telemetry export is off.
    pub fn from_env() -> Option<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let get = |name: &str| get(name).filter(|v| !v.trim().is_empty());
        if get("OTEL_SDK_DISABLED").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
            return None;
        }

        let signal_enabled = |signal: &str, exporter_var: &str| {
            let has_endpoint = get("OTEL_EXPORTER_OTLP_ENDPOINT").is_some()
                || get(&format!("OTEL_EXPORTER_OTLP_{signal}_ENDPOINT")).is_some();
            let disabled = get(exporter_var).is_some_and(|v| v.trim() == "none");
            has_endpoint && !disabled
        };
        let traces = signal_enabled("TRACES", "OTEL_TRACES_EXPORTER");
        let metrics = signal_enabled("METRICS", "OTEL_METRICS_EXPORTER");
        let logs = signal_enabled("LOGS", "OTEL_LOGS_EXPORTER");
        if !(traces || metrics || logs) {
            return None;
        }

        let protocol = match get("OTEL_EXPORTER_OTLP_PROTOCOL").as_deref() {
            None | Some("http/protobuf") => OtlpProtocol::HttpProtobuf,
            Some("grpc") => OtlpProtocol::Grpc,
            Some(other) => {
                warn!("Unsupported OTEL_EXPORTER_OTLP_PROTOCOL {other:?}, using http/protobuf");
                OtlpProtocol::HttpProtobuf
            }
        };
        Some(Self {
            protocol,
            traces,
            metrics,
            logs,
        })
    }
}

/// Flushes and stops the exporters. Keep it alive for the life of the process
/// and call [`TelemetryGuard::shutdown`] before exiting.
pub struct TelemetryGuard {
    tracer: Option<SdkTracerProvider>,
    meter: Option<SdkMeterProvider>,
    logger: Option<SdkLoggerProvider>,
}

impl TelemetryGuard {
    fn disabled() -> Self {
        Self {
            tracer: None,
            meter: None,
            logger: None,
        }
    }

    /// Whether any signal is being exported.
    pub fn is_enabled(&self) -> bool {
        self.tracer.is_some() || self.meter.is_some() || self.logger.is_some()
    }

    /// Flush pending data and stop the exporters. Blocks while exporting.
    pub fn shutdown(self) {
        if let Some(tracer) = self.tracer {
            if let Err(e) = tracer.shutdown() {
                eprintln!("Failed to flush traces: {e}");
            }
        }
        if let Some(meter) = self.meter {
            if let Err(e) = meter.shutdown() {
                eprintln!("Failed to flush metrics: {e}");
            }
        }
        if let Some(logger) = self.logger {
            if let Err(e) = logger.shutdown() {
                eprintln!("Failed to flush logs: {e}");
            }
        }
    }
}

fn resource() -> Resource {
    let builder = Resource::builder();
    let builder = if std::env::var_os("OTEL_SERVICE_NAME").is_none() {
        builder.with_service_name(DEFAULT_SERVICE_NAME)
    } else {
        builder
    };
    builder
        .with_attribute(opentelemetry::KeyValue::new(
            "service.version",
            env!("CARGO_PKG_VERSION"),
        ))
        .build()
}

type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;

/// Install the global tracing subscriber: stdout logs always, plus OTLP traces,
/// metrics and logs when configured. Call once, early in `main`.
pub fn init() -> TelemetryGuard {
    // OTEL_* settings may live in .env during development.
    let _ = dotenvy::dotenv();
    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // stderr: in `mcp --stdio` mode stdout carries the protocol.
    let mut layers: Vec<BoxedLayer> = vec![tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filter())
        .boxed()];

    let Some(settings) = TelemetrySettings::from_env() else {
        tracing_subscriber::registry().with(layers).init();
        return TelemetryGuard::disabled();
    };

    let mut guard = TelemetryGuard::disabled();
    let mut problems = Vec::new();

    // Meter provider first so the log and trace processors can report their own metrics.
    if settings.metrics {
        match build_meter_provider(&settings) {
            Ok(provider) => {
                global::set_meter_provider(provider.clone());
                guard.meter = Some(provider);
            }
            Err(e) => problems.push(format!("metrics: {e}")),
        }
    }

    if settings.logs {
        match build_logger_provider(&settings) {
            Ok(provider) => {
                // Keep the exporter's own transport out of the log export path.
                let export_filter = EnvFilter::new("info")
                    .add_directive("hyper=off".parse().expect("valid directive"))
                    .add_directive("h2=off".parse().expect("valid directive"))
                    .add_directive("tonic=off".parse().expect("valid directive"))
                    .add_directive("reqwest=off".parse().expect("valid directive"))
                    .add_directive("opentelemetry=off".parse().expect("valid directive"));
                layers.push(
                    OpenTelemetryTracingBridge::new(&provider)
                        .with_filter(export_filter)
                        .boxed(),
                );
                guard.logger = Some(provider);
            }
            Err(e) => problems.push(format!("logs: {e}")),
        }
    }

    if settings.traces {
        match build_tracer_provider(&settings) {
            Ok(provider) => {
                global::set_text_map_propagator(TraceContextPropagator::new());
                layers.push(
                    tracing_opentelemetry::layer()
                        .with_tracer(provider.tracer(DEFAULT_SERVICE_NAME))
                        .with_filter(filter())
                        .boxed(),
                );
                global::set_tracer_provider(provider.clone());
                guard.tracer = Some(provider);
            }
            Err(e) => problems.push(format!("traces: {e}")),
        }
    }

    tracing_subscriber::registry().with(layers).init();
    for problem in problems {
        warn!("OpenTelemetry export disabled for {problem}");
    }
    guard
}

fn build_tracer_provider(
    settings: &TelemetrySettings,
) -> Result<SdkTracerProvider, Box<dyn std::error::Error>> {
    let exporter = match settings.protocol {
        OtlpProtocol::HttpProtobuf => SpanExporter::builder().with_http().build()?,
        OtlpProtocol::Grpc => SpanExporter::builder().with_tonic().build()?,
    };
    Ok(SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource())
        .build())
}

fn build_meter_provider(
    settings: &TelemetrySettings,
) -> Result<SdkMeterProvider, Box<dyn std::error::Error>> {
    let exporter = match settings.protocol {
        OtlpProtocol::HttpProtobuf => MetricExporter::builder().with_http().build()?,
        OtlpProtocol::Grpc => MetricExporter::builder().with_tonic().build()?,
    };
    Ok(SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .with_resource(resource())
        .build())
}

fn build_logger_provider(
    settings: &TelemetrySettings,
) -> Result<SdkLoggerProvider, Box<dyn std::error::Error>> {
    let exporter = match settings.protocol {
        OtlpProtocol::HttpProtobuf => LogExporter::builder().with_http().build()?,
        OtlpProtocol::Grpc => LogExporter::builder().with_tonic().build()?,
    };
    Ok(SdkLoggerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource())
        .build())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn settings(vars: &[(&str, &str)]) -> Option<TelemetrySettings> {
        let vars: HashMap<_, _> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        TelemetrySettings::from_lookup(|name| vars.get(name).cloned())
    }

    #[test]
    fn off_without_an_endpoint() {
        assert_eq!(settings(&[]), None);
        assert_eq!(settings(&[("OTEL_SERVICE_NAME", "x")]), None);
        assert_eq!(settings(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "  ")]), None);
    }

    #[test]
    fn init_exports_nothing_without_an_endpoint() {
        // `init` installs a global subscriber, so test the decision it makes.
        assert!(TelemetrySettings::from_lookup(|_| None).is_none());
        assert!(!TelemetryGuard::disabled().is_enabled());
    }

    #[test]
    fn sdk_disabled_wins_over_the_endpoint() {
        assert_eq!(
            settings(&[
                ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4318"),
                ("OTEL_SDK_DISABLED", "true"),
            ]),
            None
        );
    }

    #[test]
    fn endpoint_enables_all_signals_over_http_by_default() {
        let s = settings(&[("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4318")]).unwrap();
        assert_eq!(s.protocol, OtlpProtocol::HttpProtobuf);
        assert!(s.traces && s.metrics && s.logs);
    }

    #[test]
    fn protocol_grpc_is_recognized() {
        let s = settings(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://collector:4317"),
            ("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc"),
        ])
        .unwrap();
        assert_eq!(s.protocol, OtlpProtocol::Grpc);
    }

    #[test]
    fn per_signal_endpoint_and_exporter_none() {
        let s = settings(&[(
            "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
            "http://t:4318/v1/traces",
        )])
        .unwrap();
        assert!(s.traces && !s.metrics && !s.logs);

        let s = settings(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://c:4318"),
            ("OTEL_LOGS_EXPORTER", "none"),
        ])
        .unwrap();
        assert!(s.traces && s.metrics && !s.logs);
    }
}
