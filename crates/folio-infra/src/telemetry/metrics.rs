//! Application metrics
//!
//! Instruments come from the global meter provider, so they are no-ops until
//! [`super::init`] installs one. Create them after that call.

use std::sync::OnceLock;
use std::time::Instant;

use opentelemetry::metrics::{Counter, Histogram, UpDownCounter};
use opentelemetry::{global, KeyValue};

pub struct Metrics {
    jobs_processed: Counter<u64>,
    jobs_failed: Counter<u64>,
    job_duration: Histogram<f64>,
    documents: UpDownCounter<i64>,
    ai_duration: Histogram<f64>,
    storage_duration: Histogram<f64>,
}

/// The process-wide instruments.
pub fn metrics() -> &'static Metrics {
    static METRICS: OnceLock<Metrics> = OnceLock::new();
    METRICS.get_or_init(|| {
        let meter = global::meter("folio");
        Metrics {
            jobs_processed: meter
                .u64_counter("folio.jobs.processed")
                .with_description("Background jobs that finished an attempt")
                .build(),
            jobs_failed: meter
                .u64_counter("folio.jobs.failed")
                .with_description("Background job attempts that failed")
                .build(),
            job_duration: meter
                .f64_histogram("folio.jobs.duration")
                .with_unit("s")
                .with_description("Duration of one job attempt")
                .build(),
            documents: meter
                .i64_up_down_counter("folio.documents")
                .with_description("Documents currently stored")
                .build(),
            ai_duration: meter
                .f64_histogram("folio.ai.request.duration")
                .with_unit("s")
                .with_description("Latency of OCR and embedding calls to AI providers")
                .build(),
            storage_duration: meter
                .f64_histogram("folio.storage.operation.duration")
                .with_unit("s")
                .with_description("Latency of document store operations")
                .build(),
        }
    })
}

impl Metrics {
    /// Record one finished job attempt.
    pub fn job_finished(&self, job_type: &'static str, success: bool, started: Instant) {
        let attrs = [KeyValue::new("job.type", job_type)];
        self.job_duration
            .record(started.elapsed().as_secs_f64(), &attrs);
        self.jobs_processed.add(
            1,
            &[
                KeyValue::new("job.type", job_type),
                KeyValue::new("outcome", if success { "success" } else { "failure" }),
            ],
        );
        if !success {
            self.jobs_failed.add(1, &attrs);
        }
    }

    /// Adjust the stored-document count.
    pub fn documents_changed(&self, delta: i64) {
        self.documents.add(delta, &[]);
    }

    /// Record an OCR or embedding call (`operation` is `ocr` or `embedding`).
    pub fn ai_request(
        &self,
        operation: &'static str,
        provider: &'static str,
        success: bool,
        started: Instant,
    ) {
        self.ai_duration.record(
            started.elapsed().as_secs_f64(),
            &[
                KeyValue::new("operation", operation),
                KeyValue::new("provider", provider),
                KeyValue::new("outcome", if success { "success" } else { "failure" }),
            ],
        );
    }

    /// Record a document store operation.
    pub fn storage_operation(
        &self,
        operation: &'static str,
        backend: &'static str,
        started: Instant,
    ) {
        self.storage_duration.record(
            started.elapsed().as_secs_f64(),
            &[
                KeyValue::new("operation", operation),
                KeyValue::new("backend", backend),
            ],
        );
    }
}
