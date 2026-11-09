//! Background job worker

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::watch;
use tokio::time::sleep;
use tracing::{debug, error, info, info_span, warn, Instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use folio_core::events::{DocumentEvent, EventOrigin};
use folio_core::models::{ProcessingStage, ProcessingStatus, VaultId};
use folio_core::ports::{
    DocumentRepository, DocumentStore, EmbeddedChunk, EmbeddingService, OcrService, OutboxStore,
    SearchService,
};

use super::{FailOutcome, Job, JobClass, JobQueue, JobType, HEARTBEAT_SECONDS};
use crate::ai::chunker::{chunk_text, DEFAULT_CHUNK_CHARS, DEFAULT_OVERLAP_CHARS};
use crate::ai::PipelineContext;
use crate::telemetry::metrics;
use crate::thumbnail::ThumbnailService;
use crate::workflow::{RunOutcome, WorkflowRunner};

/// Chunks sent to the embedding provider per request
const EMBED_BATCH_SIZE: usize = 16;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, thiserror::Error)]
#[error("{source}")]
struct ProcessingFailure {
    reason: folio_core::processing_events::ProcessingReason,
    #[source]
    source: BoxError,
}

fn processing_failure(
    reason: folio_core::processing_events::ProcessingReason,
    source: impl Into<BoxError>,
) -> BoxError {
    Box::new(ProcessingFailure {
        reason,
        source: source.into(),
    })
}

async fn run_with_lease<T>(
    queue: &JobQueue,
    job: &Job,
    work: impl Future<Output = T>,
    period: Duration,
    renewal_timeout: Duration,
) -> Option<T> {
    if !renew_claim(queue, job, renewal_timeout).await {
        return None;
    }
    let heartbeats = async {
        let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            heartbeat.tick().await;
            if !renew_claim(queue, job, renewal_timeout).await {
                return;
            }
        }
    };
    tokio::select! {
        biased;
        _ = heartbeats => None,
        result = work => Some(result),
    }
}

/// A first attempt that runs right away continues the trace that queued it.
/// Retries and delayed jobs start their own trace with a link back, so a
/// request's trace does not keep growing for hours.
fn relate_to_queuing_trace(span: &tracing::Span, job: &Job) {
    let Some(trace) = &job.trace else { return };
    let immediate = job.scheduled_at - job.created_at <= PARENT_WINDOW;
    if job.attempts <= 1 && immediate {
        if let Err(e) = span.set_parent(trace.to_parent()) {
            debug!(job.id = %job.id, "Cannot continue the queuing trace: {e}");
        }
    } else if let Some(queued) = trace.span_context() {
        span.add_link(queued);
    }
}

/// Longest delay between queuing and scheduled start for which a job still
/// joins the queuing trace
const PARENT_WINDOW: chrono::TimeDelta = chrono::TimeDelta::minutes(1);

async fn renew_claim(queue: &JobQueue, job: &Job, timeout: Duration) -> bool {
    match tokio::time::timeout(timeout, queue.renew(job)).await {
        Ok(Ok(owned)) => owned,
        Ok(Err(error)) => {
            error!(job.id = %job.id, %error, "Failed to renew job lease");
            false
        }
        Err(_) => {
            error!(job.id = %job.id, "Job lease renewal timed out");
            false
        }
    }
}

/// Configuration for the job worker
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// How often to poll for jobs (in milliseconds)
    pub poll_interval_ms: u64,
    /// How often to reset stuck jobs (in seconds)
    pub stuck_check_interval_secs: u64,
    /// How often to check for orphaned documents (in seconds)
    pub repair_check_interval_secs: u64,
    /// Workflow runs executed at the same time, apart from OCR and thumbnails
    pub workflow_concurrency: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1000,
            stuck_check_interval_secs: 60,
            repair_check_interval_secs: 300, // Every 5 minutes
            workflow_concurrency: 4,
        }
    }
}

/// Background job worker
pub struct Worker {
    queue: JobQueue,
    document_repo: Arc<dyn DocumentRepository>,
    ocr_service: Arc<dyn OcrService>,
    embedding_service: Arc<dyn EmbeddingService>,
    search_service: Arc<dyn SearchService>,
    pipeline: Arc<PipelineContext>,
    thumbnail_service: Arc<ThumbnailService>,
    store: Arc<dyn DocumentStore>,
    config: WorkerConfig,
    outbox: Arc<dyn OutboxStore>,
    workflow_runner: Option<Arc<WorkflowRunner>>,
}

impl Worker {
    /// Create a new worker
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        queue: JobQueue,
        document_repo: Arc<dyn DocumentRepository>,
        ocr_service: Arc<dyn OcrService>,
        embedding_service: Arc<dyn EmbeddingService>,
        search_service: Arc<dyn SearchService>,
        pipeline: Arc<PipelineContext>,
        thumbnail_service: Arc<ThumbnailService>,
        store: Arc<dyn DocumentStore>,
        config: WorkerConfig,
        outbox: Arc<dyn OutboxStore>,
    ) -> Self {
        Self {
            queue,
            document_repo,
            ocr_service,
            embedding_service,
            search_service,
            pipeline,
            thumbnail_service,
            store,
            config,
            outbox,
            workflow_runner: None,
        }
    }

    /// Let this worker execute workflow runs
    pub fn with_workflow_runner(mut self, runner: Arc<WorkflowRunner>) -> Self {
        self.workflow_runner = Some(runner);
        self
    }

    /// Persist an event, to be announced to clients and workflows by the dispatcher
    async fn emit(
        &self,
        vault: VaultId,
        event: DocumentEvent,
        origin: EventOrigin,
    ) -> Result<(), BoxError> {
        self.outbox.append_document(vault, &event, origin).await?;
        Ok(())
    }

    /// Like [`emit`](Self::emit) for a progress hint that must not fail the step it follows
    async fn emit_logged(&self, vault: VaultId, event: DocumentEvent) {
        if let Err(e) = self.emit(vault, event, EventOrigin::User).await {
            error!("Failed to persist event: {e}");
        }
    }

    /// Start the worker (runs until shutdown signal)
    ///
    /// Document processing and workflow runs are separate loops, so a
    /// webhook target that never answers cannot hold up OCR.
    pub async fn run(&self, shutdown: watch::Receiver<bool>) {
        info!("Job worker started");
        tokio::join!(
            self.run_processing(shutdown.clone()),
            self.run_workflows(shutdown)
        );
    }

    async fn run_processing(&self, mut shutdown: watch::Receiver<bool>) {
        // Run repair immediately on startup to catch any orphaned documents
        if let Err(e) = self.repair_orphaned_documents().await {
            error!("Failed initial document repair: {}", e);
        }

        let mut stuck_check_counter = 0u64;
        let mut repair_check_counter = 0u64;

        loop {
            // Check for shutdown
            if *shutdown.borrow() {
                info!("Job worker shutting down");
                break;
            }

            // Periodically check for stuck jobs
            stuck_check_counter += self.config.poll_interval_ms;
            if stuck_check_counter >= self.config.stuck_check_interval_secs * 1000 {
                stuck_check_counter = 0;
                if let Err(e) = self.queue.reset_stuck().await {
                    error!("Failed to reset stuck jobs: {}", e);
                }
            }

            // Periodically repair orphaned documents (documents without content and without jobs)
            repair_check_counter += self.config.poll_interval_ms;
            if repair_check_counter >= self.config.repair_check_interval_secs * 1000 {
                repair_check_counter = 0;
                if let Err(e) = self.repair_orphaned_documents().await {
                    error!("Failed to repair orphaned documents: {}", e);
                }
            }

            // Try to get a job
            match self.queue.dequeue_class(JobClass::Processing).await {
                Ok(Some(job)) => self.handle_job(job).await,
                Ok(None) => {
                    // No jobs available, wait before polling again
                    tokio::select! {
                        _ = sleep(Duration::from_millis(self.config.poll_interval_ms)) => {}
                        _ = shutdown.changed() => {
                            if *shutdown.borrow() {
                                info!("Job worker shutting down");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to dequeue job: {}", e);
                    sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
                }
            }
        }
    }

    /// Execute workflow runs, several at a time
    async fn run_workflows(&self, mut shutdown: watch::Receiver<bool>) {
        let poll = Duration::from_millis(self.config.poll_interval_ms);
        let mut in_flight = FuturesUnordered::new();
        loop {
            if *shutdown.borrow() {
                break;
            }
            while in_flight.len() < self.config.workflow_concurrency {
                match self.queue.dequeue_class(JobClass::Workflow).await {
                    Ok(Some(job)) => in_flight.push(self.handle_job(job)),
                    Ok(None) => break,
                    Err(e) => {
                        error!("Failed to dequeue workflow job: {}", e);
                        break;
                    }
                }
            }
            if in_flight.is_empty() {
                tokio::select! {
                    _ = sleep(poll) => {}
                    _ = shutdown.changed() => {}
                }
            } else {
                tokio::select! {
                    _ = in_flight.next() => {}
                    _ = sleep(poll) => {}
                    _ = shutdown.changed() => {}
                }
            }
        }
        while in_flight.next().await.is_some() {}
    }

    /// Run a claimed job and record how it went
    async fn handle_job(&self, job: Job) {
        debug!("Processing job {}: {:?}", job.id, job.job_type.type_name());

        let type_name = job.job_type.type_name();
        let span = info_span!(
            "job",
            otel.name = %format!("job {type_name}"),
            otel.kind = "consumer",
            job.id = %job.id,
            job.r#type = type_name,
            job.attempt = job.attempts,
            document.id = job.job_type.document_id().map(tracing::field::display),
        );
        relate_to_queuing_trace(&span, &job);
        let started = Instant::now();
        let Some(result) = run_with_lease(
            &self.queue,
            &job,
            self.process_claim(&job).instrument(span.clone()),
            Duration::from_secs(HEARTBEAT_SECONDS),
            Duration::from_secs(HEARTBEAT_SECONDS),
        )
        .await
        else {
            warn!(job.id = %job.id, "Stopped job after losing its lease");
            return;
        };
        metrics().job_finished(type_name, result.is_ok(), started);

        match result {
            Ok(()) => match self.queue.complete(&job).instrument(span.clone()).await {
                Ok(true) => {}
                Ok(false) => warn!(job.id = %job.id, "Completion rejected after losing job lease"),
                Err(e) => error!("Failed to mark job {} as complete: {}", job.id, e),
            },
            Err(e) => {
                let error_msg = e.to_string();
                match self
                    .queue
                    .fail_with_reason(
                        &job,
                        &error_msg,
                        true,
                        e.downcast_ref::<ProcessingFailure>().map_or(
                            folio_core::processing_events::ProcessingReason::ProviderFailed,
                            |failure| failure.reason,
                        ),
                    )
                    .instrument(span.clone())
                    .await
                {
                    Ok(FailOutcome::GaveUp) => self.abandon_run(&job.job_type, &error_msg).await,
                    Ok(FailOutcome::Retrying) => {}
                    Ok(FailOutcome::LostLease) => {
                        warn!(job.id = %job.id, "Failure rejected after losing job lease")
                    }
                    Err(e) => error!("Failed to mark job {} as failed: {}", job.id, e),
                }
            }
        }
    }

    /// A workflow job that gave up leaves its run pending forever unless the run is closed
    async fn abandon_run(&self, job_type: &JobType, reason: &str) {
        let (JobType::ExecuteWorkflowRun { run_id }, Some(runner)) =
            (job_type, &self.workflow_runner)
        else {
            return;
        };
        if let Err(e) = runner.abandon(*run_id, reason).await {
            error!("Failed to mark workflow run {run_id} as failed: {e}");
        }
    }

    /// Re-queue documents whose OCR or embedding stage is still pending but has no active job
    ///
    /// Driven by the per-stage status, never by whether `content` is set, so a
    /// document with legitimately empty OCR output is not picked up again.
    async fn repair_orphaned_documents(&self) -> Result<(), BoxError> {
        for document in self.document_repo.list_purge_requested().await? {
            let active = self
                .queue
                .find_active_jobs_for_document(document.id)
                .await?;
            if !active
                .iter()
                .any(|job| matches!(job.job_type, JobType::PurgeDocument { .. }))
            {
                let job = self.queue.latest_purge_job(document.id).await?.unwrap_or(
                    JobType::PurgeDocument {
                        document_id: document.id,
                        correlation_id: None,
                        causation_id: None,
                    },
                );
                self.queue.enqueue(job).await?;
            }
        }

        let mut repaired_count = 0;

        for doc in self
            .document_repo
            .list_pending(ProcessingStage::Ocr)
            .await?
        {
            if doc.file_path.is_none() {
                continue;
            }
            if self.has_active_job(doc.id).await? {
                continue;
            }
            info!("Repairing orphaned document {} ({})", doc.id, doc.title);
            self.queue
                .enqueue(JobType::process_document(doc.id))
                .await?;
            repaired_count += 1;
        }

        for doc in self
            .document_repo
            .list_pending(ProcessingStage::Embed)
            .await?
        {
            if doc.content.as_deref().is_some_and(|c| !c.is_empty()) {
                if self.has_active_job(doc.id).await? {
                    continue;
                }
                self.queue
                    .enqueue(JobType::generate_embedding(doc.id))
                    .await?;
                repaired_count += 1;
            } else if doc.ocr_status != ProcessingStatus::Pending
                && doc.ocr_status != ProcessingStatus::Running
            {
                // OCR is over and left no text, so there is nothing to embed.
                self.document_repo
                    .set_stage_status(
                        doc.vault_id,
                        doc.id,
                        ProcessingStage::Embed,
                        ProcessingStatus::Skipped,
                    )
                    .await?;
            }
        }

        if repaired_count > 0 {
            info!("Repaired {} orphaned documents", repaired_count);
        }

        Ok(())
    }

    /// The vault a job's document lives in; jobs carry only the document id
    async fn vault_of(&self, document_id: uuid::Uuid) -> Result<VaultId, BoxError> {
        Ok(self
            .document_repo
            .locate(document_id)
            .await?
            .ok_or("Document not found")?)
    }

    async fn has_active_job(&self, document_id: uuid::Uuid) -> Result<bool, BoxError> {
        Ok(!self
            .queue
            .find_active_jobs_for_document(document_id)
            .await?
            .is_empty())
    }

    async fn process_claim(&self, job: &Job) -> Result<(), BoxError> {
        let scope = match super::processing_scope::prepare_claim(&self.queue, job).await? {
            super::processing_scope::ClaimPreparation::Ready(scope) => scope,
            super::processing_scope::ClaimPreparation::Obsolete => {
                if let Some(document_id) = job.job_type.document_id() {
                    self.purge_if_deleted(document_id).await?;
                }
                return Ok(());
            }
        };
        let context = self.queue.context(job.id).await?;
        if let (Some(vault), Some(document)) = (
            context.and_then(|context| context.vault_id),
            job.job_type.document_id(),
        ) {
            if self
                .document_repo
                .locate(document)
                .await?
                .is_some_and(|current| current != vault)
            {
                warn!(job.id = %job.id, %document, "Skipping job for a document moved to another vault");
                return Ok(());
            }
        }
        folio_core::mutation::with_actor(
            None,
            folio_core::mutation::with_lineage(
                context.map(|context| context.lineage),
                super::processing_scope::with_scope(
                    scope,
                    Box::pin(self.process_job_with_claim(&job.job_type, Some(job))),
                ),
            ),
        )
        .await
    }

    #[cfg(test)]
    async fn process_job(&self, job_type: &JobType) -> Result<(), BoxError> {
        self.process_job_with_claim(job_type, None).await
    }

    async fn process_job_with_claim(
        &self,
        job_type: &JobType,
        claim: Option<&Job>,
    ) -> Result<(), BoxError> {
        if let Some(document_id) = job_type.document_id() {
            if !matches!(job_type, JobType::PurgeDocument { .. })
                && self.document_repo.locate(document_id).await?.is_none()
            {
                self.purge_if_deleted(document_id).await?;
                return Ok(());
            }
        }
        let result = if matches!(job_type, JobType::ExtractText { .. }) {
            match claim {
                Some(job) => Box::pin(self.process_claimed_ocr(job)).await,
                None => Box::pin(self.run_job(job_type)).await,
            }
        } else {
            Box::pin(self.run_job(job_type)).await
        };
        if let Some(document_id) = job_type.document_id() {
            if let Err(e) = self.purge_if_deleted(document_id).await {
                error!("Failed to purge leftovers of deleted document {document_id}: {e}");
            }
        }
        result
    }

    /// Fail the job when the document is gone, so a delete is not followed by new writes
    async fn ensure_exists(&self, document_id: uuid::Uuid) -> Result<(), BoxError> {
        match self.document_repo.locate(document_id).await? {
            Some(_) => Ok(()),
            None => Err(format!("Document {document_id} was deleted").into()),
        }
    }

    /// Remove stored objects a job wrote after its document was deleted.
    /// The check before each write narrows that race; this closes it.
    /// Chunks need no purge: they cascade with the document and `store_chunks`
    /// refuses a document that no longer exists.
    async fn purge_if_deleted(&self, document_id: uuid::Uuid) -> Result<(), BoxError> {
        if self
            .document_repo
            .locate_retained(document_id)
            .await?
            .is_some()
        {
            return Ok(());
        }
        folio_core::ports::purge_document(self.store.as_ref(), document_id).await?;
        Ok(())
    }

    async fn run_job(&self, job_type: &JobType) -> Result<(), BoxError> {
        match job_type {
            JobType::GenerateThumbnail { document_id } => {
                self.process_thumbnail(*document_id).await
            }
            JobType::ExtractText { document_id, force } => {
                self.process_ocr(*document_id, *force).await
            }
            JobType::GenerateEmbedding {
                document_id,
                origin,
            } => self.process_embedding(*document_id, *origin).await,
            JobType::ProcessDocument { document_id, force } => {
                self.process_document(*document_id, *force).await
            }
            JobType::ExecuteWorkflowRun { run_id } => self.execute_workflow_run(*run_id).await,
            JobType::PurgeDocument {
                document_id,
                correlation_id,
                causation_id,
            } => {
                let lineage =
                    correlation_id.map(|correlation_id| folio_core::mutation::MutationLineage {
                        correlation_id,
                        causation_id: *causation_id,
                    });
                folio_core::mutation::with_actor(
                    None,
                    folio_core::mutation::with_lineage(lineage, self.purge_requested(*document_id)),
                )
                .await
            }
        }
    }

    async fn purge_requested(&self, document_id: uuid::Uuid) -> Result<(), BoxError> {
        let Some(vault) = self.document_repo.locate_trashed(document_id).await? else {
            return Ok(());
        };
        let Some(document) = self
            .document_repo
            .find_trashed_by_id(vault, document_id)
            .await?
        else {
            return Ok(());
        };
        if document.purge_requested_at.is_none() {
            return Ok(());
        }
        let active = self
            .queue
            .find_active_jobs_for_document(document_id)
            .await?;
        if active.iter().any(|job| {
            job.status == super::JobStatus::Processing
                && !matches!(job.job_type, JobType::PurgeDocument { .. })
        }) {
            return Err("document processing is still finishing".into());
        }
        folio_core::ports::purge_document(self.store.as_ref(), document_id).await?;
        self.document_repo
            .finalize_purge(vault, document_id)
            .await?;
        Ok(())
    }

    /// Run a workflow; a retryable failure is reported as an error so the queue backs off
    async fn execute_workflow_run(
        &self,
        run_id: uuid::Uuid,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let runner = self
            .workflow_runner
            .as_ref()
            .ok_or("Workflow runner is not configured")?;
        match runner.execute(run_id).await? {
            RunOutcome::Retry => Err("Workflow action failed, will retry".into()),
            RunOutcome::Succeeded | RunOutcome::Failed => Ok(()),
        }
    }

    /// Process thumbnail generation
    async fn process_thumbnail(
        &self,
        document_id: uuid::Uuid,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let vault = self.vault_of(document_id).await?;
        let document = self
            .document_repo
            .find_by_id(vault, document_id)
            .await?
            .ok_or("Document not found")?;

        // Read the file
        let file_path = document.file_path.as_ref().ok_or("No file path")?;
        let data = self.store.get_bytes(file_path).await?;

        let mime_type = document
            .mime_type
            .as_deref()
            .unwrap_or("application/octet-stream");

        self.ensure_exists(document_id).await?;
        self.thumbnail_service
            .generate(document_id, &data, mime_type, document.content.as_deref())
            .await?;

        info!("Generated thumbnail for document {}", document_id);

        // Emit event
        self.emit(
            vault,
            DocumentEvent::ThumbnailReady { document_id },
            EventOrigin::User,
        )
        .await?;

        Ok(())
    }

    /// Run a stage, marking it running first and failed if it errors
    async fn track<T>(
        &self,
        vault: VaultId,
        document_id: uuid::Uuid,
        stage: ProcessingStage,
        work: impl std::future::Future<Output = Result<T, BoxError>>,
    ) -> Result<T, BoxError> {
        self.document_repo
            .set_stage_status(vault, document_id, stage, ProcessingStatus::Running)
            .await?;
        match work.await {
            Ok(value) => Ok(value),
            Err(e) => {
                if let Err(status_err) = self
                    .document_repo
                    .set_stage_status(vault, document_id, stage, ProcessingStatus::Failed)
                    .await
                {
                    error!("Failed to record {:?} failure: {}", stage, status_err);
                }
                Err(e)
            }
        }
    }

    async fn process_claimed_ocr(&self, job: &Job) -> Result<(), BoxError> {
        use folio_core::processing_events::ProcessingReason;
        let Some(fence) = self.queue.start_ocr(job).await? else {
            return Ok(());
        };
        let force = matches!(job.job_type, JobType::ExtractText { force: true, .. });
        let settings = self.pipeline.settings().await;
        let embed = settings.processing.auto_embed && settings.embeddings.provider != "none";
        let document = self
            .document_repo
            .find_by_id(fence.vault_id(), fence.document_id())
            .await?
            .ok_or("Document not found")?;
        if let Some(content) = document
            .content
            .as_deref()
            .filter(|content| !force && !content.is_empty())
        {
            self.queue
                .complete_ocr(&fence, content, embed)
                .await
                .map_err(|error| processing_failure(ProcessingReason::StorageFailed, error))?;
            return Ok(());
        }
        if settings.ocr.provider == "none" {
            self.queue
                .skip_ocr(&fence, embed)
                .await
                .map_err(|error| processing_failure(ProcessingReason::StorageFailed, error))?;
            return Ok(());
        }
        let path = document
            .file_path
            .as_ref()
            .ok_or_else(|| processing_failure(ProcessingReason::StorageFailed, "No file path"))?;
        let data = self
            .store
            .get_bytes(path)
            .await
            .map_err(|error| processing_failure(ProcessingReason::StorageFailed, error))?;
        let mime_type = document
            .mime_type
            .as_deref()
            .unwrap_or("application/octet-stream");
        let output = self
            .ocr_service
            .extract(&data, mime_type)
            .await
            .map_err(|error| processing_failure(ProcessingReason::ProviderFailed, error))?;
        self.queue
            .complete_ocr(&fence, &output.text, embed)
            .await
            .map_err(|error| processing_failure(ProcessingReason::StorageFailed, error))?;
        Ok(())
    }

    /// Process OCR text extraction
    async fn process_ocr(&self, document_id: uuid::Uuid, force: bool) -> Result<(), BoxError> {
        let vault = self.vault_of(document_id).await?;
        let document = self
            .document_repo
            .find_by_id(vault, document_id)
            .await?
            .ok_or("Document not found")?;

        if !force && document.ocr_status == ProcessingStatus::Done {
            let has_text = document
                .content
                .as_deref()
                .is_some_and(|content| !content.is_empty());
            return self.schedule_embedding(vault, document_id, has_text).await;
        }

        let set_status = |status| {
            self.document_repo
                .set_stage_status(vault, document_id, ProcessingStage::Ocr, status)
        };

        if let Some(content) = document
            .content
            .as_deref()
            .filter(|content| !force && !content.is_empty())
        {
            debug!("Document {} already has content, skipping OCR", document_id);
            self.document_repo
                .complete_ocr(vault, document_id, content)
                .await?;
            return self.schedule_embedding(vault, document_id, true).await;
        }

        if self.pipeline.settings().await.ocr.provider == "none" {
            set_status(ProcessingStatus::Skipped).await?;
            return self.schedule_embedding(vault, document_id, false).await;
        }

        let output = self
            .track(vault, document_id, ProcessingStage::Ocr, async {
                let file_path = document.file_path.as_ref().ok_or("No file path")?;
                let data = self.store.get_bytes(file_path).await?;
                let mime_type = document
                    .mime_type
                    .as_deref()
                    .unwrap_or("application/octet-stream");
                Ok(self.ocr_service.extract(&data, mime_type).await?)
            })
            .await?;

        // An empty result is a successful OCR: store "" so the document is not retried.
        self.document_repo
            .complete_ocr(vault, document_id, &output.text)
            .await?;

        info!(
            "OCR finished for document {} (engine={}, {} chars, confidence={:?})",
            document_id,
            output.engine,
            output.text.len(),
            output.confidence
        );

        let has_text = !output.text.is_empty();
        self.schedule_embedding(vault, document_id, has_text).await
    }

    /// End the embedding stage, the last step of the pipeline, and announce that processing finished
    async fn finish_embed_stage(
        &self,
        vault: VaultId,
        document_id: uuid::Uuid,
        status: ProcessingStatus,
        origin: EventOrigin,
    ) -> Result<(), BoxError> {
        self.document_repo
            .set_stage_status(vault, document_id, ProcessingStage::Embed, status)
            .await?;
        self.emit(vault, DocumentEvent::Processed { document_id }, origin)
            .await?;
        Ok(())
    }

    /// Queue the embedding stage, or mark it skipped when it cannot or should not run
    async fn schedule_embedding(
        &self,
        vault: VaultId,
        document_id: uuid::Uuid,
        has_text: bool,
    ) -> Result<(), BoxError> {
        let settings = self.pipeline.settings().await;
        if has_text && settings.processing.auto_embed && settings.embeddings.provider != "none" {
            self.document_repo
                .set_stage_status(
                    vault,
                    document_id,
                    ProcessingStage::Embed,
                    ProcessingStatus::Pending,
                )
                .await?;
            self.queue
                .enqueue(JobType::generate_embedding(document_id))
                .await?;
        } else {
            self.finish_embed_stage(
                vault,
                document_id,
                ProcessingStatus::Skipped,
                EventOrigin::User,
            )
            .await?;
        }
        Ok(())
    }

    /// Process embedding generation
    async fn process_embedding(
        &self,
        document_id: uuid::Uuid,
        origin: EventOrigin,
    ) -> Result<(), BoxError> {
        let vault = self.vault_of(document_id).await?;
        let document = self
            .document_repo
            .find_by_id(vault, document_id)
            .await?
            .ok_or("Document not found")?;

        let content = document.content.unwrap_or_default();
        let chunks = chunk_text(&content, DEFAULT_CHUNK_CHARS, DEFAULT_OVERLAP_CHARS);
        if chunks.is_empty() {
            self.finish_embed_stage(vault, document_id, ProcessingStatus::Skipped, origin)
                .await?;
            return Ok(());
        }

        if self.pipeline.settings().await.embeddings.provider == "none" {
            self.finish_embed_stage(vault, document_id, ProcessingStatus::Skipped, origin)
                .await?;
            return Ok(());
        }

        self.track(vault, document_id, ProcessingStage::Embed, async {
            let active_model = self.embedding_service.model_id().await?;
            self.search_service.mark_stale_except(&active_model).await?;

            let mut embedded = Vec::with_capacity(chunks.len());
            let mut model = active_model;
            for (batch_no, batch_texts) in chunks.chunks(EMBED_BATCH_SIZE).enumerate() {
                let texts: Vec<&str> = batch_texts.iter().map(String::as_str).collect();
                let batch = self.embedding_service.embed(&texts).await?;
                batch.validate(texts.len())?;
                model = batch.model;
                for (i, (text, embedding)) in batch_texts.iter().zip(batch.vectors).enumerate() {
                    embedded.push(EmbeddedChunk {
                        index: (batch_no * EMBED_BATCH_SIZE + i) as u32,
                        text: text.clone(),
                        embedding,
                    });
                }
            }

            self.ensure_exists(document_id).await?;
            self.search_service
                .store_chunks(vault, document_id, &model, &embedded)
                .await?;
            self.finish_embed_stage(vault, document_id, ProcessingStatus::Done, origin)
                .await?;
            info!(
                "Stored {} embedded chunks for document {} (model {})",
                embedded.len(),
                document_id,
                model
            );
            Ok(())
        })
        .await
    }

    /// Process a newly uploaded document: thumbnails, then queue the OCR stage
    async fn process_document(&self, document_id: uuid::Uuid, force: bool) -> Result<(), BoxError> {
        info!("Starting document processing for {}", document_id);

        let vault = self.vault_of(document_id).await?;
        let document = self
            .document_repo
            .find_by_id(vault, document_id)
            .await?
            .ok_or("Document not found")?;

        info!(
            "Processing document: id={}, title={}, mime_type={:?}",
            document_id, document.title, document.mime_type
        );

        let file_path = match &document.file_path {
            Some(p) => p.clone(),
            None => {
                error!("Document {} has no file path", document_id);
                return Err("No file path".into());
            }
        };

        let mime_type = document
            .mime_type
            .as_deref()
            .unwrap_or("application/octet-stream");

        // PDFs go through poppler, which needs a file: fetch them to disk once
        // and reuse that copy for the page count and every page.
        info!("Reading file from: {}", file_path);
        let local_pdf = if mime_type == "application/pdf" {
            Some(self.store.download_to_temp(&file_path).await?)
        } else {
            None
        };
        let data = match &local_pdf {
            Some(local) => tokio::fs::read(local.path()).await?,
            None => self.store.get_bytes(&file_path).await?,
        };
        info!("File read successfully, size: {} bytes", data.len());

        // Step 1: Generate thumbnail(s) and get page count for PDFs
        let (_, page_count) = if let Some(local_pdf) = &local_pdf {
            // Get page count
            let count = match self
                .thumbnail_service
                .get_pdf_page_count(local_pdf.path())
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    debug!("Failed to get PDF page count: {}", e);
                    1
                }
            };

            // Store page count
            if let Err(e) = self
                .document_repo
                .set_page_count(vault, document_id, count)
                .await
            {
                debug!("Failed to set page count: {}", e);
            }

            let max_pages = count.min(folio_core::ports::PREVIEW_PAGE_LIMIT);
            let mut success = false;

            for page in 1..=max_pages {
                self.ensure_exists(document_id).await?;
                match self
                    .thumbnail_service
                    .generate_page(document_id, local_pdf.path(), page)
                    .await
                {
                    Ok(_) => {
                        if page == 1 {
                            self.emit_logged(vault, DocumentEvent::ThumbnailReady { document_id })
                                .await;
                            success = true;
                        }
                    }
                    Err(e) => {
                        debug!("Thumbnail generation failed for page {}: {}", page, e);
                    }
                }
            }

            (success, Some(count))
        } else {
            // Non-PDF: use original single-thumbnail generation
            self.ensure_exists(document_id).await?;
            let ok = match self
                .thumbnail_service
                .generate(document_id, &data, mime_type, document.content.as_deref())
                .await
            {
                Ok(_) => {
                    self.emit_logged(vault, DocumentEvent::ThumbnailReady { document_id })
                        .await;
                    true
                }
                Err(e) => {
                    debug!("Thumbnail generation failed: {}", e);
                    false
                }
            };

            // Images have 1 page
            if mime_type.starts_with("image/") {
                if let Err(e) = self
                    .document_repo
                    .set_page_count(vault, document_id, 1)
                    .await
                {
                    debug!("Failed to set page count: {}", e);
                }
            }

            (ok, None)
        };

        // Log page count if available
        if let Some(count) = page_count {
            debug!("Document {} has {} pages", document_id, count);
        }

        // Steps 2 and 3 run as their own jobs so each can be retried and
        // tracked independently: OCR, then embedding.
        let settings = self.pipeline.settings().await;
        let has_text = document.content.as_deref().is_some_and(|c| !c.is_empty());
        if force || has_text || settings.processing.auto_ocr {
            self.document_repo
                .set_stage_status(
                    vault,
                    document_id,
                    ProcessingStage::Ocr,
                    ProcessingStatus::Pending,
                )
                .await?;
            self.queue
                .enqueue(JobType::ExtractText { document_id, force })
                .await?;
        } else {
            self.document_repo
                .set_stage_status(
                    vault,
                    document_id,
                    ProcessingStage::Ocr,
                    ProcessingStatus::Skipped,
                )
                .await?;
            self.schedule_embedding(vault, document_id, false).await?;
        }

        info!("Completed processing document {}", document_id);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AiConfig;
    use crate::database::{
        test_pool, SqliteDocumentRepository, SqliteOutboxStore, SqliteSettingsRepository,
        SqliteWorkflowRepository,
    };
    use crate::workflow::WebhookClient;
    use crate::SqliteSearchService;
    use async_trait::async_trait;
    use folio_core::error::Result;
    use folio_core::models::{
        CreateDocument, RunStatus, WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun,
        WorkflowTrigger,
    };
    use folio_core::ports::{EmbeddingBatch, OcrOutput, SettingsRepository, WorkflowRepository};

    struct FixedOcr(&'static str);

    #[async_trait]
    impl OcrService for FixedOcr {
        async fn extract(&self, _: &[u8], _: &str) -> Result<OcrOutput> {
            Ok(OcrOutput::from_pages("fixed", vec![self.0.to_string()]))
        }
        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    struct FixedEmbeddings;

    #[async_trait]
    impl EmbeddingService for FixedEmbeddings {
        async fn embed(&self, texts: &[&str]) -> Result<EmbeddingBatch> {
            Ok(EmbeddingBatch {
                vectors: texts.iter().map(|_| vec![1.0, 0.0]).collect(),
                model: "test:model".to_string(),
                dimensions: 2,
            })
        }
        async fn model_id(&self) -> Result<String> {
            Ok("test:model".to_string())
        }
        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    struct MissingEmbeddings;

    #[async_trait]
    impl EmbeddingService for MissingEmbeddings {
        async fn embed(&self, _: &[&str]) -> Result<EmbeddingBatch> {
            Ok(EmbeddingBatch {
                vectors: Vec::new(),
                model: "test:model".into(),
                dimensions: 2,
            })
        }
        async fn model_id(&self) -> Result<String> {
            Ok("test:model".into())
        }
        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn missing_embedding_vectors_never_publish_completion() {
        let mut f = fixture("invoice").await;
        f.worker.embedding_service = Arc::new(MissingEmbeddings);
        f.settings
            .set("embeddings.provider", "openai")
            .await
            .unwrap();
        f.pipeline.invalidate().await;
        f.repo
            .complete_ocr(crate::database::test_vault(), f.doc_id, "invoice")
            .await
            .unwrap();

        assert!(f
            .worker
            .process_embedding(f.doc_id, EventOrigin::User)
            .await
            .is_err());

        let doc = f
            .repo
            .find_by_id(crate::database::test_vault(), f.doc_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.embed_status, ProcessingStatus::Failed);
        let (chunks,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM document_chunks WHERE document_id = ?")
                .bind(f.doc_id.to_string())
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(chunks, 0);
        let (completed,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM outbox_events WHERE type = 'processed'")
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(completed, 0);
    }

    struct Fixture {
        pool: sqlx::SqlitePool,
        worker: Worker,
        repo: Arc<SqliteDocumentRepository>,
        settings: Arc<SqliteSettingsRepository>,
        search: Arc<SqliteSearchService>,
        queue: JobQueue,
        pipeline: Arc<PipelineContext>,
        doc_id: uuid::Uuid,
        storage: Arc<dyn DocumentStore>,
        _dir: tempfile::TempDir,
    }

    async fn fixture(ocr_text: &'static str) -> Fixture {
        let (pool, dir) = test_pool().await;
        let repo = Arc::new(SqliteDocumentRepository::new(pool.clone()));
        let settings = Arc::new(SqliteSettingsRepository::new(pool.clone()));
        let search = Arc::new(SqliteSearchService::new(pool.clone()));
        let queue = JobQueue::sqlite(pool.clone());
        let pipeline = PipelineContext::new(
            settings.clone(),
            AiConfig {
                provider: "local".into(),
                openai_api_key: None,
                anthropic_api_key: None,
            },
        );
        let storage: Arc<dyn DocumentStore> =
            Arc::new(crate::storage::ObjectStoreDocumentStore::in_memory());
        let thumbnails = Arc::new(ThumbnailService::new(storage.clone()));
        let doc = repo
            .create(
                crate::database::test_vault(),
                CreateDocument {
                    title: "scan".into(),
                    content: None,
                    original_filename: Some("scan.png".into()),
                    mime_type: Some("image/png".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let path = folio_core::ports::keys::original(doc.id);
        storage
            .put(&path, b"not a real png".as_slice().into())
            .await
            .unwrap();
        repo.set_file_path(crate::database::test_vault(), doc.id, &path)
            .await
            .unwrap();

        let worker = Worker::new(
            queue.clone(),
            repo.clone(),
            Arc::new(FixedOcr(ocr_text)),
            Arc::new(FixedEmbeddings),
            search.clone(),
            pipeline.clone(),
            thumbnails,
            storage.clone(),
            WorkerConfig::default(),
            Arc::new(SqliteOutboxStore::new(pool.clone())),
        );

        Fixture {
            pool,
            worker,
            repo,
            settings,
            search,
            queue,
            pipeline,
            doc_id: doc.id,
            storage,
            _dir: dir,
        }
    }

    #[tokio::test]
    async fn processing_claim_restores_source_lineage_without_inheriting_actor() {
        use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
        let f = fixture("OCR").await;
        let lineage = MutationLineage {
            correlation_id: uuid::Uuid::new_v4(),
            causation_id: Some(uuid::Uuid::new_v4()),
        };
        with_lineage(
            Some(lineage),
            f.queue.enqueue(JobType::extract_text(f.doc_id)),
        )
        .await
        .unwrap();
        let job = f.queue.dequeue().await.unwrap().unwrap();
        with_actor(
            Some(folio_core::models::UserId("fixture-user".into())),
            f.worker.process_claim(&job),
        )
        .await
        .unwrap();
        let payload: String =
            sqlx::query_scalar("SELECT payload FROM outbox_events WHERE type = 'ocr_complete'")
                .fetch_one(&f.pool)
                .await
                .unwrap();
        let event = folio_core::events::decode_stored_outbox_payload(&payload)
            .unwrap()
            .envelope
            .unwrap();
        assert_eq!(event.correlation_id(), lineage.correlation_id);
        assert_eq!(event.causation_id(), lineage.causation_id);
        assert_eq!(event.actor_id(), None);
    }

    #[tokio::test]
    async fn processing_claim_does_not_start_in_a_different_vault() {
        let f = fixture("OCR").await;
        f.queue
            .enqueue(JobType::extract_text(f.doc_id))
            .await
            .unwrap();
        let job = f.queue.dequeue().await.unwrap().unwrap();
        let destination = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO vaults(id, name, created_at, updated_at) VALUES (?, 'Moved', '2026-11-03T00:00:00Z', '2026-11-03T00:00:00Z')")
            .bind(destination.to_string()).execute(&f.pool).await.unwrap();
        sqlx::query("UPDATE documents SET vault_id = ? WHERE id = ?")
            .bind(destination.to_string())
            .bind(f.doc_id.to_string())
            .execute(&f.pool)
            .await
            .unwrap();
        f.worker.process_claim(&job).await.unwrap();
        let state: (String, Option<String>) =
            sqlx::query_as("SELECT ocr_status, content FROM documents WHERE id = ?")
                .bind(f.doc_id.to_string())
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(state, ("pending".into(), None));
        let events: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM outbox_events WHERE type = 'ocr_complete'")
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(events, 0);
    }

    #[tokio::test]
    async fn queued_processing_does_not_write_into_a_deleted_vault() {
        let f = fixture("OCR").await;
        f.queue
            .enqueue(JobType::extract_text(f.doc_id))
            .await
            .unwrap();
        let job = f.queue.dequeue().await.unwrap().unwrap();
        sqlx::query(
            "UPDATE vaults SET deleted_at = '2026-11-03T00:00:00Z'
             WHERE id = (SELECT vault_id FROM documents WHERE id = ?)",
        )
        .bind(f.doc_id.to_string())
        .execute(&f.pool)
        .await
        .unwrap();
        f.worker.process_claim(&job).await.unwrap();
        let content: Option<String> =
            sqlx::query_scalar("SELECT content FROM documents WHERE id = ?")
                .bind(f.doc_id.to_string())
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(content, None);
        let chunks: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM document_chunks WHERE document_id = ?")
                .bind(f.doc_id.to_string())
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(chunks, 0);
    }

    async fn drain(f: &Fixture) {
        while let Some(job) = f.queue.dequeue().await.unwrap() {
            f.worker.process_job(&job.job_type).await.unwrap();
            f.queue.complete(&job).await.unwrap();
        }
    }

    async fn drain_claimed(f: &Fixture) {
        while let Some(job) = f.queue.dequeue().await.unwrap() {
            f.worker.process_claim(&job).await.unwrap();
            f.queue.complete(&job).await.unwrap();
        }
    }

    struct FailingOcr;

    #[async_trait]
    impl OcrService for FailingOcr {
        async fn extract(&self, _: &[u8], _: &str) -> Result<OcrOutput> {
            Err(folio_core::Error::AiService("provider down".into()))
        }
        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    async fn processed_with_text(f: &Fixture, text: &str) {
        f.settings
            .set("embeddings.provider", "openai")
            .await
            .unwrap();
        f.pipeline.invalidate().await;
        f.repo
            .update(
                crate::database::test_vault(),
                f.doc_id,
                folio_core::models::UpdateDocument {
                    content: Some(text.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        f.queue
            .enqueue(JobType::process_document(f.doc_id))
            .await
            .unwrap();
        drain_claimed(f).await;
    }

    async fn chunk_texts(f: &Fixture) -> Vec<String> {
        sqlx::query_scalar("SELECT text FROM document_chunks WHERE document_id = ? AND stale = 0")
            .bind(f.doc_id.to_string())
            .fetch_all(&f.pool)
            .await
            .unwrap()
    }

    /// A PDF of `pages` blank pages; poppler rebuilds the missing cross-reference table
    fn blank_pdf(pages: u32) -> Vec<u8> {
        let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", i + 3)).collect();
        let mut pdf = format!(
            "%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
             2 0 obj << /Type /Pages /Kids [{}] /Count {pages} >> endobj\n",
            kids.join(" ")
        );
        for i in 0..pages {
            pdf.push_str(&format!(
                "{} 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >> endobj\n",
                i + 3
            ));
        }
        pdf.push_str("trailer << /Root 1 0 R >>\n%%EOF\n");
        pdf.into_bytes()
    }

    #[tokio::test]
    async fn long_pdfs_get_previews_for_the_leading_pages_only() {
        use folio_core::ports::{keys, PREVIEW_PAGE_LIMIT};
        let f = fixture("text").await;
        let vault = crate::database::test_vault();
        let pages = PREVIEW_PAGE_LIMIT + 1;
        let doc = f
            .repo
            .create(
                vault,
                CreateDocument {
                    title: "long".into(),
                    mime_type: Some("application/pdf".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let path = keys::original(doc.id);
        f.storage.put(&path, blank_pdf(pages).into()).await.unwrap();
        f.repo.set_file_path(vault, doc.id, &path).await.unwrap();

        for force in [false, true] {
            f.queue
                .enqueue(JobType::ProcessDocument {
                    document_id: doc.id,
                    force,
                })
                .await
                .unwrap();
            drain_claimed(&f).await;

            let stored = f.repo.find_by_id(vault, doc.id).await.unwrap().unwrap();
            assert_eq!(stored.page_count, Some(pages));
            assert!(f
                .storage
                .exists(&keys::page(doc.id, PREVIEW_PAGE_LIMIT))
                .await
                .unwrap());
            assert!(!f.storage.exists(&keys::page(doc.id, pages)).await.unwrap());
        }
    }

    #[tokio::test]
    async fn forced_reprocess_replaces_existing_text_and_reembeds() {
        let f = fixture("fresh OCR text").await;
        processed_with_text(&f, "old text").await;
        assert_eq!(chunk_texts(&f).await, vec!["old text".to_string()]);

        f.queue
            .enqueue(JobType::ProcessDocument {
                document_id: f.doc_id,
                force: true,
            })
            .await
            .unwrap();
        drain_claimed(&f).await;

        let doc = f
            .repo
            .find_by_id(crate::database::test_vault(), f.doc_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.content.as_deref(), Some("fresh OCR text"));
        assert_eq!(doc.ocr_status, ProcessingStatus::Done);
        assert_eq!(doc.embed_status, ProcessingStatus::Done);
        assert_eq!(chunk_texts(&f).await, vec!["fresh OCR text".to_string()]);
    }

    #[tokio::test]
    async fn forced_ocr_failure_keeps_existing_text() {
        let mut f = fixture("unused").await;
        processed_with_text(&f, "old text").await;
        f.worker.ocr_service = Arc::new(FailingOcr);

        f.queue
            .enqueue(JobType::ExtractText {
                document_id: f.doc_id,
                force: true,
            })
            .await
            .unwrap();
        let job = f.queue.dequeue().await.unwrap().unwrap();
        assert!(f.worker.process_claim(&job).await.is_err());

        let doc = f
            .repo
            .find_by_id(crate::database::test_vault(), f.doc_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.content.as_deref(), Some("old text"));
        assert_eq!(chunk_texts(&f).await, vec!["old text".to_string()]);
    }

    #[test]
    fn job_payloads_without_force_stay_unforced() {
        let id = uuid::Uuid::new_v4();
        let legacy = format!(r#"{{"type":"ProcessDocument","data":{{"document_id":"{id}"}}}}"#);
        let parsed: JobType = serde_json::from_str(&legacy).unwrap();
        assert!(matches!(
            parsed,
            JobType::ProcessDocument { force: false, .. }
        ));
        let unforced = JobType::extract_text(id);
        assert!(!serde_json::to_string(&unforced).unwrap().contains("force"));
    }

    #[tokio::test]
    async fn long_work_renews_its_lease_while_pending() {
        let f = fixture("OCR").await;
        f.queue
            .enqueue(JobType::extract_text(f.doc_id))
            .await
            .unwrap();
        let job = f.queue.dequeue().await.unwrap().unwrap();
        let shortened_expiry = job.lease_expires_at.unwrap() - 200;
        let work = async {
            sqlx::query("UPDATE jobs SET lease_expires_at = ? WHERE id = ?")
                .bind(shortened_expiry)
                .bind(job.id.to_string())
                .execute(&f.pool)
                .await
                .unwrap();
            loop {
                let (expiry,): (i64,) =
                    sqlx::query_as("SELECT lease_expires_at FROM jobs WHERE id = ?")
                        .bind(job.id.to_string())
                        .fetch_one(&f.pool)
                        .await
                        .unwrap();
                if expiry > shortened_expiry {
                    assert_eq!(f.queue.reset_stuck().await.unwrap(), 0);
                    return;
                }
                tokio::task::yield_now().await;
            }
        };
        assert!(tokio::time::timeout(
            Duration::from_secs(5),
            run_with_lease(
                &f.queue,
                &job,
                work,
                Duration::from_millis(100),
                Duration::from_secs(5)
            )
        )
        .await
        .unwrap()
        .is_some());
    }

    #[tokio::test]
    async fn pending_renewal_does_not_stop_work_releasing_its_connection() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::database::run_migrations(&pool).await.unwrap();
        let queue = JobQueue::sqlite(pool.clone());
        queue
            .enqueue(JobType::extract_text(uuid::Uuid::new_v4()))
            .await
            .unwrap();
        let job = queue.dequeue().await.unwrap().unwrap();
        let shortened_expiry = job.lease_expires_at.unwrap() - 200;
        let work = async {
            let mut connection = pool.acquire().await.unwrap();
            sqlx::query("UPDATE jobs SET lease_expires_at = ?")
                .bind(shortened_expiry)
                .execute(&mut *connection)
                .await
                .unwrap();
            sleep(Duration::from_millis(200)).await;
            drop(connection);
            loop {
                let (expiry,): (i64,) =
                    sqlx::query_as("SELECT lease_expires_at FROM jobs WHERE id = ?")
                        .bind(job.id.to_string())
                        .fetch_one(&pool)
                        .await
                        .unwrap();
                if expiry > shortened_expiry {
                    return;
                }
                tokio::task::yield_now().await;
            }
        };
        assert!(tokio::time::timeout(
            Duration::from_secs(5),
            run_with_lease(
                &queue,
                &job,
                work,
                Duration::from_millis(100),
                Duration::from_secs(5)
            ),
        )
        .await
        .unwrap()
        .is_some());
    }

    #[tokio::test]
    async fn renewal_timeout_cancels_work_and_releases_its_connection() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::database::run_migrations(&pool).await.unwrap();
        let queue = JobQueue::sqlite(pool.clone());
        queue
            .enqueue(JobType::extract_text(uuid::Uuid::new_v4()))
            .await
            .unwrap();
        let job = queue.dequeue().await.unwrap().unwrap();
        let started = std::cell::Cell::new(false);
        let work = async {
            let _connection = pool.acquire().await.unwrap();
            started.set(true);
            std::future::pending::<()>().await;
        };
        assert!(tokio::time::timeout(
            Duration::from_secs(5),
            run_with_lease(
                &queue,
                &job,
                work,
                Duration::from_millis(100),
                Duration::from_millis(200)
            ),
        )
        .await
        .unwrap()
        .is_none());
        assert!(started.get());
        assert!(
            tokio::time::timeout(Duration::from_secs(5), queue.renew(&job))
                .await
                .unwrap()
                .unwrap()
        );
    }

    #[tokio::test]
    async fn expired_claim_does_not_start_work() {
        let f = fixture("OCR").await;
        f.queue
            .enqueue(JobType::extract_text(f.doc_id))
            .await
            .unwrap();
        let job = f.queue.dequeue().await.unwrap().unwrap();
        sqlx::query("UPDATE jobs SET lease_expires_at = 0 WHERE id = ?")
            .bind(job.id.to_string())
            .execute(&f.pool)
            .await
            .unwrap();
        let work = async {
            panic!("expired claim must never start work");
        };
        assert!(run_with_lease(
            &f.queue,
            &job,
            work,
            Duration::from_secs(60),
            Duration::from_secs(5)
        )
        .await
        .is_none());
    }

    #[tokio::test]
    async fn losing_a_lease_drops_pending_work() {
        struct DropFlag(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let f = fixture("OCR").await;
        f.queue
            .enqueue(JobType::extract_text(f.doc_id))
            .await
            .unwrap();
        let job = f.queue.dequeue().await.unwrap().unwrap();
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let guard = DropFlag(dropped.clone());
        let work = async {
            let _guard = guard;
            sqlx::query("UPDATE jobs SET lease_expires_at = 0 WHERE id = ?")
                .bind(job.id.to_string())
                .execute(&f.pool)
                .await
                .unwrap();
            std::future::pending::<()>().await;
        };
        assert!(tokio::time::timeout(
            Duration::from_secs(5),
            run_with_lease(
                &f.queue,
                &job,
                work,
                Duration::from_millis(100),
                Duration::from_secs(5)
            )
        )
        .await
        .unwrap()
        .is_none());
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn empty_ocr_result_is_stored_and_not_requeued() {
        let f = fixture("").await;
        f.queue
            .enqueue(JobType::process_document(f.doc_id))
            .await
            .unwrap();
        drain(&f).await;

        let doc = f
            .repo
            .find_by_id(crate::database::test_vault(), f.doc_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.content.as_deref(), Some(""));
        assert_eq!(doc.ocr_status, ProcessingStatus::Done);
        assert_eq!(doc.embed_status, ProcessingStatus::Skipped);

        f.worker.repair_orphaned_documents().await.unwrap();
        f.worker.repair_orphaned_documents().await.unwrap();
        assert_eq!(f.queue.pending_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn retry_after_empty_ocr_completion_does_not_repeat_ocr_or_its_event() {
        let mut f = fixture("").await;
        f.worker.process_ocr(f.doc_id, false).await.unwrap();
        f.worker.ocr_service = Arc::new(FixedOcr("unexpected recomputation"));

        f.worker.process_ocr(f.doc_id, false).await.unwrap();

        let doc = f
            .repo
            .find_by_id(crate::database::test_vault(), f.doc_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.content.as_deref(), Some(""));
        let (events,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM outbox_events WHERE type = 'ocr_complete'")
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(events, 1);
    }

    #[tokio::test]
    async fn repair_requeues_documents_that_were_never_processed() {
        let f = fixture("x").await;
        f.worker.repair_orphaned_documents().await.unwrap();
        assert_eq!(f.queue.pending_count().await.unwrap(), 1);
        f.worker.repair_orphaned_documents().await.unwrap();
        assert_eq!(f.queue.pending_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn ocr_then_embedding_run_in_sequence_and_store_model_tagged_chunks() {
        let f = fixture("Invoice from ACME for services").await;
        f.settings
            .set("embeddings.provider", "openai")
            .await
            .unwrap();
        f.pipeline.invalidate().await;

        f.queue
            .enqueue(JobType::process_document(f.doc_id))
            .await
            .unwrap();
        drain(&f).await;

        let doc = f
            .repo
            .find_by_id(crate::database::test_vault(), f.doc_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            doc.content.as_deref(),
            Some("Invoice from ACME for services")
        );
        assert_eq!(doc.ocr_status, ProcessingStatus::Done);
        assert_eq!(doc.embed_status, ProcessingStatus::Done);

        let hits = f
            .search
            .search(crate::database::test_vault(), "test:model", &[1.0, 0.0], 5)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert!(f
            .search
            .search(crate::database::test_vault(), "other:model", &[1.0, 0.0], 5)
            .await
            .unwrap()
            .is_empty());
    }

    /// Storage that deletes the document row the first time its original is read,
    /// the way a user delete lands in the middle of a running job.
    struct DeleteOnRead {
        inner: Arc<dyn DocumentStore>,
        repo: Arc<SqliteDocumentRepository>,
        doc_id: uuid::Uuid,
    }

    #[async_trait]
    impl DocumentStore for DeleteOnRead {
        fn backend(&self) -> &'static str {
            self.inner.backend()
        }
        async fn put(&self, key: &str, data: bytes::Bytes) -> Result<()> {
            self.inner.put(key, data).await
        }
        async fn get(&self, key: &str) -> Result<folio_core::ports::StoredObject> {
            let object = self.inner.get(key).await?;
            let _ = self
                .repo
                .delete(crate::database::test_vault(), self.doc_id)
                .await;
            Ok(object)
        }
        async fn size(&self, key: &str) -> Result<Option<u64>> {
            self.inner.size(key).await
        }
        async fn exists(&self, key: &str) -> Result<bool> {
            self.inner.exists(key).await
        }
        async fn delete(&self, key: &str) -> Result<()> {
            self.inner.delete(key).await
        }
        async fn delete_prefix(&self, prefix: &str) -> Result<u64> {
            self.inner.delete_prefix(prefix).await
        }
        async fn list(&self, prefix: &str) -> Result<Vec<String>> {
            self.inner.list(prefix).await
        }
    }

    #[tokio::test]
    async fn a_document_trashed_mid_job_keeps_its_original() {
        let mut f = fixture("text").await;
        f.worker.store = Arc::new(DeleteOnRead {
            inner: f.storage.clone(),
            repo: f.repo.clone(),
            doc_id: f.doc_id,
        });
        let mut png = Vec::new();
        image::RgbImage::new(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        f.storage
            .put(&folio_core::ports::keys::original(f.doc_id), png.into())
            .await
            .unwrap();

        f.worker
            .process_job(&JobType::process_document(f.doc_id))
            .await
            .ok();

        assert!(f
            .storage
            .exists(&folio_core::ports::keys::original(f.doc_id))
            .await
            .unwrap());
        assert!(!f
            .storage
            .exists(&folio_core::ports::keys::thumbnail(f.doc_id))
            .await
            .unwrap());
        assert!(f
            .repo
            .find_trashed_by_id(crate::database::test_vault(), f.doc_id)
            .await
            .unwrap()
            .is_some());
    }

    struct UnavailableDeleteStore {
        inner: Arc<dyn DocumentStore>,
    }

    #[async_trait]
    impl DocumentStore for UnavailableDeleteStore {
        fn backend(&self) -> &'static str {
            self.inner.backend()
        }
        async fn put(&self, key: &str, data: bytes::Bytes) -> Result<()> {
            self.inner.put(key, data).await
        }
        async fn get(&self, key: &str) -> Result<folio_core::ports::StoredObject> {
            self.inner.get(key).await
        }
        async fn size(&self, key: &str) -> Result<Option<u64>> {
            self.inner.size(key).await
        }
        async fn exists(&self, key: &str) -> Result<bool> {
            self.inner.exists(key).await
        }
        async fn delete(&self, key: &str) -> Result<()> {
            self.inner.delete(key).await
        }
        async fn delete_prefix(&self, _prefix: &str) -> Result<u64> {
            Err(folio_core::error::Error::Storage(
                "storage unavailable".into(),
            ))
        }
        async fn list(&self, prefix: &str) -> Result<Vec<String>> {
            self.inner.list(prefix).await
        }
    }

    #[tokio::test]
    async fn failed_permanent_deletion_keeps_durable_intent_and_repair_retries_it() {
        let mut f = fixture("text").await;
        let vault = crate::database::test_vault();
        f.worker.store = Arc::new(UnavailableDeleteStore {
            inner: f.storage.clone(),
        });
        f.repo.delete(vault, f.doc_id).await.unwrap();
        f.repo.request_purge(vault, f.doc_id).await.unwrap();
        let job = f
            .queue
            .dequeue_class(JobClass::Processing)
            .await
            .unwrap()
            .unwrap();

        let original_payload = serde_json::to_value(&job.job_type).unwrap();
        assert!(original_payload["data"]["causation_id"].is_string());
        f.worker.handle_job(job).await;

        assert!(f.repo.find_by_id(vault, f.doc_id).await.unwrap().is_none());
        assert!(f
            .repo
            .find_trashed_by_id(vault, f.doc_id)
            .await
            .unwrap()
            .unwrap()
            .purge_requested_at
            .is_some());
        assert!(f
            .storage
            .exists(&folio_core::ports::keys::original(f.doc_id))
            .await
            .unwrap());
        sqlx::query("UPDATE jobs SET status = 'failed'")
            .execute(&f.pool)
            .await
            .unwrap();
        f.worker.repair_orphaned_documents().await.unwrap();
        let retry = f
            .queue
            .dequeue_class(JobClass::Processing)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(retry.job_type, JobType::PurgeDocument { .. }));
        assert_eq!(
            serde_json::to_value(&retry.job_type).unwrap(),
            original_payload
        );
        f.worker.store = f.storage.clone();

        folio_core::mutation::with_actor(
            Some(folio_core::models::UserId::from("unexpected-parent")),
            f.worker.handle_job(retry),
        )
        .await;
        let payloads: Vec<(String,)> = sqlx::query_as("SELECT payload FROM outbox_events")
            .fetch_all(&f.pool)
            .await
            .unwrap();
        let purged = payloads
            .iter()
            .map(|(value,)| serde_json::from_str::<serde_json::Value>(value).unwrap())
            .find(|value| value["envelope"]["event_type"] == "document.purged")
            .unwrap();
        assert_eq!(
            purged["envelope"]["correlation_id"],
            original_payload["data"]["correlation_id"]
        );
        assert_eq!(
            purged["envelope"]["causation_id"],
            original_payload["data"]["causation_id"]
        );
        assert!(purged["envelope"]["actor_id"].is_null());

        assert!(f
            .repo
            .find_trashed_by_id(vault, f.doc_id)
            .await
            .unwrap()
            .is_none());
        assert!(f
            .storage
            .list(&folio_core::ports::keys::document_prefix(f.doc_id))
            .await
            .unwrap()
            .is_empty());
        f.worker
            .process_job(&JobType::PurgeDocument {
                document_id: f.doc_id,
                correlation_id: None,
                causation_id: None,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn trash_never_expires_and_queued_processing_preserves_files() {
        let f = fixture("text").await;
        let vault = crate::database::test_vault();
        f.repo.delete(vault, f.doc_id).await.unwrap();
        sqlx::query("UPDATE documents SET deleted_at = '1970-01-01T00:00:00+00:00'")
            .execute(&f.pool)
            .await
            .unwrap();

        f.worker.repair_orphaned_documents().await.unwrap();
        f.worker
            .process_job(&JobType::process_document(f.doc_id))
            .await
            .unwrap();
        f.worker
            .process_job(&JobType::PurgeDocument {
                document_id: f.doc_id,
                correlation_id: None,
                causation_id: None,
            })
            .await
            .unwrap();

        assert_eq!(f.queue.pending_count().await.unwrap(), 0);
        assert!(f
            .repo
            .find_trashed_by_id(vault, f.doc_id)
            .await
            .unwrap()
            .unwrap()
            .purge_requested_at
            .is_none());
        assert!(f
            .storage
            .exists(&folio_core::ports::keys::original(f.doc_id))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn permanent_deletion_waits_for_running_processing_before_removing_storage() {
        let f = fixture("text").await;
        let vault = crate::database::test_vault();
        f.queue
            .enqueue(JobType::process_document(f.doc_id))
            .await
            .unwrap();
        let running = f
            .queue
            .dequeue_class(JobClass::Processing)
            .await
            .unwrap()
            .unwrap();
        f.repo.delete(vault, f.doc_id).await.unwrap();
        f.repo.request_purge(vault, f.doc_id).await.unwrap();

        assert!(f
            .worker
            .process_job(&JobType::PurgeDocument {
                document_id: f.doc_id,
                correlation_id: None,
                causation_id: None
            })
            .await
            .is_err());
        assert!(f
            .storage
            .exists(&folio_core::ports::keys::original(f.doc_id))
            .await
            .unwrap());
        f.queue.complete(&running).await.unwrap();
        f.worker
            .process_job(&JobType::PurgeDocument {
                document_id: f.doc_id,
                correlation_id: None,
                causation_id: None,
            })
            .await
            .unwrap();

        assert!(f
            .repo
            .find_trashed_by_id(vault, f.doc_id)
            .await
            .unwrap()
            .is_none());
        assert!(!f
            .storage
            .exists(&folio_core::ports::keys::original(f.doc_id))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn post_job_cleanup_preserves_objects_through_trash_and_restore() {
        let f = fixture("text").await;
        let vault = crate::database::test_vault();
        f.repo.delete(vault, f.doc_id).await.unwrap();

        f.worker.purge_if_deleted(f.doc_id).await.unwrap();
        assert!(f
            .storage
            .exists(&folio_core::ports::keys::original(f.doc_id))
            .await
            .unwrap());
        f.repo.restore(vault, f.doc_id).await.unwrap();
        f.worker.purge_if_deleted(f.doc_id).await.unwrap();

        assert!(f
            .storage
            .exists(&folio_core::ports::keys::original(f.doc_id))
            .await
            .unwrap());
        assert!(f.repo.find_by_id(vault, f.doc_id).await.unwrap().is_some());
    }

    struct DeletingEmbeddings(Arc<SqliteDocumentRepository>, uuid::Uuid);

    #[async_trait]
    impl EmbeddingService for DeletingEmbeddings {
        async fn embed(&self, texts: &[&str]) -> Result<EmbeddingBatch> {
            let _ = self.0.delete(crate::database::test_vault(), self.1).await;
            FixedEmbeddings.embed(texts).await
        }
        async fn model_id(&self) -> Result<String> {
            Ok("test:model".to_string())
        }
        async fn health(&self) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn processed_carries_the_origin_of_the_embedding_job() {
        let f = fixture("").await;
        f.repo
            .complete_ocr(crate::database::test_vault(), f.doc_id, "imported text")
            .await
            .unwrap();
        f.settings
            .set("embeddings.provider", "openai")
            .await
            .unwrap();
        f.pipeline.invalidate().await;

        for origin in [EventOrigin::Import, EventOrigin::User] {
            f.worker
                .process_job(&JobType::GenerateEmbedding {
                    document_id: f.doc_id,
                    origin,
                })
                .await
                .unwrap();
        }

        let origins: Vec<_> = SqliteOutboxStore::new(f.pool.clone())
            .replay(0, chrono::Utc::now() - chrono::Duration::minutes(5), 100)
            .await
            .unwrap()
            .into_iter()
            .filter(|event| matches!(event.event, Some(DocumentEvent::Processed { .. })))
            .map(|event| event.origin)
            .collect();
        assert_eq!(origins, vec![EventOrigin::Import, EventOrigin::User]);
    }

    #[test]
    fn embedding_jobs_without_an_origin_are_user_jobs() {
        let id = uuid::Uuid::new_v4();
        let legacy = serde_json::json!({"type": "GenerateEmbedding", "data": {"document_id": id}});
        let job: JobType = serde_json::from_value(legacy.clone()).unwrap();
        assert!(matches!(
            job,
            JobType::GenerateEmbedding { document_id, origin: EventOrigin::User } if document_id == id
        ));
        assert_eq!(serde_json::to_value(&job).unwrap(), legacy);
    }

    /// Run processing jobs, leaving queued workflow runs alone
    async fn drain_processing(f: &Fixture) {
        while let Some(job) = f.queue.dequeue_class(JobClass::Processing).await.unwrap() {
            f.worker.process_job(&job.job_type).await.unwrap();
            f.queue.complete(&job).await.unwrap();
        }
    }

    /// Hand every committed event to the workflow engine, as the dispatcher would
    async fn dispatch_to(engine: &folio_core::services::WorkflowEngine, f: &Fixture) {
        use folio_core::ports::OutboxStore;
        use folio_core::services::EventConsumer;
        let outbox = SqliteOutboxStore::new(f.pool.clone());
        for event in outbox
            .claim(1000, chrono::Duration::seconds(30), 10)
            .await
            .unwrap()
        {
            engine.consume(&event).await.unwrap();
            outbox.mark_dispatched(event.id).await.unwrap();
        }
    }

    #[tokio::test]
    async fn imports_start_no_processed_workflows_but_a_user_reprocess_does() {
        use crate::database::{SqliteCorrespondentRepository, SqliteDocumentTypeRepository};
        use folio_core::models::{ExternalRef, ImportSource, ImportedDocument};

        let f = fixture("OCR text").await;
        let vault = crate::database::test_vault();
        f.settings
            .set("embeddings.provider", "openai")
            .await
            .unwrap();
        f.pipeline.invalidate().await;
        let workflows = Arc::new(SqliteWorkflowRepository::new(f.pool.clone()));
        let engine = folio_core::services::WorkflowEngine::new(
            workflows.clone(),
            f.repo.clone(),
            Arc::new(SqliteCorrespondentRepository::new(f.pool.clone())),
            Arc::new(SqliteDocumentTypeRepository::new(f.pool.clone())),
            Arc::new(crate::StorageFileMetadata::new(f.storage.clone())),
            "http://folio.test",
        );
        let mut on = Vec::new();
        for trigger in [
            WorkflowTrigger::DocumentUploaded,
            WorkflowTrigger::DocumentProcessed,
        ] {
            on.push(
                workflows
                    .create(
                        vault,
                        WorkflowInput {
                            name: trigger.as_str().into(),
                            enabled: true,
                            trigger,
                            conditions: WorkflowConditions::default(),
                            actions: vec![WorkflowAction::Webhook {
                                url: "http://127.0.0.1:1/hook".into(),
                                method: Default::default(),
                                headers: Default::default(),
                                secret: None,
                            }],
                        },
                    )
                    .await
                    .unwrap(),
            );
        }
        let id = uuid::Uuid::new_v4();
        let runs = |index: usize| {
            let workflows = workflows.clone();
            let workflow = on[index].id;
            async move {
                workflows
                    .list_runs(vault, workflow, 10)
                    .await
                    .unwrap()
                    .iter()
                    .filter(|run| run.document_id == Some(id))
                    .count()
            }
        };
        let path = folio_core::ports::keys::original(id);
        f.storage
            .put(&path, b"not a real png".as_slice().into())
            .await
            .unwrap();
        let now = chrono::Utc::now();
        f.repo
            .create_imported(
                vault,
                ImportedDocument {
                    id,
                    external_ref: ExternalRef {
                        source: ImportSource::Paperless,
                        id: "7".into(),
                    },
                    title: "Imported".into(),
                    content: Some("imported text".into()),
                    original_filename: Some("scan.png".into()),
                    mime_type: Some("image/png".into()),
                    file_path: path,
                    content_sha256: "ab".repeat(32),
                    document_date: None,
                    created_at: now,
                    updated_at: now,
                    correspondent_id: None,
                    document_type_id: None,
                    tag_ids: Vec::new(),
                    metadata: Default::default(),
                    notes: Vec::new(),
                    in_inbox: false,
                },
            )
            .await
            .unwrap();
        // A repair sweep right after the import must not queue a user-origin twin.
        f.worker.repair_orphaned_documents().await.unwrap();
        drain_processing(&f).await;
        dispatch_to(&engine, &f).await;
        assert_eq!(runs(0).await, 0, "uploaded");
        assert_eq!(runs(1).await, 0, "processed");
        let imported = f.repo.find_by_id(vault, id).await.unwrap().unwrap();
        assert_eq!(imported.embed_status, ProcessingStatus::Done);

        f.queue
            .enqueue(JobType::ProcessDocument {
                document_id: id,
                force: false,
            })
            .await
            .unwrap();
        drain_processing(&f).await;
        dispatch_to(&engine, &f).await;
        assert_eq!(runs(1).await, 1, "a user reprocess triggers processed");
        assert_eq!(runs(0).await, 0);
    }

    #[tokio::test]
    async fn a_document_deleted_mid_job_gets_no_embedding() {
        let mut f = fixture("some text").await;
        f.worker.embedding_service = Arc::new(DeletingEmbeddings(f.repo.clone(), f.doc_id));
        f.repo
            .complete_ocr(crate::database::test_vault(), f.doc_id, "some text")
            .await
            .unwrap();
        f.settings
            .set("embeddings.provider", "openai")
            .await
            .unwrap();
        f.pipeline.invalidate().await;

        f.worker
            .process_job(&JobType::generate_embedding(f.doc_id))
            .await
            .ok();

        assert!(f
            .search
            .search(crate::database::test_vault(), "test:model", &[1.0, 0.0], 5)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn auto_flags_are_honoured() {
        let f = fixture("some text").await;
        f.settings
            .set("embeddings.provider", "openai")
            .await
            .unwrap();
        f.settings
            .set("processing.auto_embed", "false")
            .await
            .unwrap();
        f.pipeline.invalidate().await;

        f.queue
            .enqueue(JobType::process_document(f.doc_id))
            .await
            .unwrap();
        drain(&f).await;

        let doc = f
            .repo
            .find_by_id(crate::database::test_vault(), f.doc_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.ocr_status, ProcessingStatus::Done);
        assert_eq!(doc.embed_status, ProcessingStatus::Skipped);

        f.settings
            .set("processing.auto_ocr", "false")
            .await
            .unwrap();
        f.pipeline.invalidate().await;
        let doc2 = f
            .repo
            .create(
                crate::database::test_vault(),
                CreateDocument {
                    title: "second".into(),
                    content: None,
                    original_filename: None,
                    mime_type: Some("image/png".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let path = folio_core::ports::keys::original(doc2.id);
        f.storage.put(&path, b"x".as_slice().into()).await.unwrap();
        f.repo
            .set_file_path(crate::database::test_vault(), doc2.id, &path)
            .await
            .unwrap();
        f.worker
            .process_job(&JobType::process_document(doc2.id))
            .await
            .unwrap();
        let doc2 = f
            .repo
            .find_by_id(crate::database::test_vault(), doc2.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc2.ocr_status, ProcessingStatus::Skipped);
    }

    #[tokio::test]
    async fn failed_ocr_is_recorded_and_not_requeued_by_repair() {
        let f = fixture("x").await;
        f.repo
            .set_file_path(crate::database::test_vault(), f.doc_id, "missing/file.png")
            .await
            .unwrap();

        assert!(f
            .worker
            .process_job(&JobType::extract_text(f.doc_id))
            .await
            .is_err());

        let doc = f
            .repo
            .find_by_id(crate::database::test_vault(), f.doc_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.ocr_status, ProcessingStatus::Failed);
        f.worker.repair_orphaned_documents().await.unwrap();
        assert_eq!(f.queue.pending_count().await.unwrap(), 0);
    }

    /// Give the fixture's worker a workflow runner and queue a run that cannot succeed
    async fn queue_unreachable_run(
        f: &mut Fixture,
    ) -> (Arc<SqliteWorkflowRepository>, WorkflowRun) {
        let workflows = Arc::new(SqliteWorkflowRepository::new(f.pool.clone()));
        let runner = Arc::new(WorkflowRunner::new(
            workflows.clone(),
            f.repo.clone(),
            WebhookClient::new(),
        ));
        f.worker.workflow_runner = Some(runner);

        let workflow = workflows
            .create(
                crate::database::test_vault(),
                WorkflowInput {
                    name: "hook".into(),
                    enabled: true,
                    trigger: WorkflowTrigger::DocumentUploaded,
                    conditions: WorkflowConditions::default(),
                    actions: vec![WorkflowAction::Webhook {
                        url: "http://127.0.0.1:1/hook".into(),
                        method: Default::default(),
                        headers: Default::default(),
                        secret: None,
                    }],
                },
            )
            .await
            .unwrap();
        let run = WorkflowRun::new(
            workflow.vault_id,
            workflow.id,
            WorkflowTrigger::DocumentUploaded,
            Some(f.doc_id),
            "{}".into(),
        )
        .with_actions(workflow.actions);
        assert!(workflows.create_run(&run).await.unwrap());
        (workflows, run)
    }

    #[tokio::test]
    async fn a_run_fails_when_its_job_gives_up() {
        let mut f = fixture("text").await;
        let (workflows, run) = queue_unreachable_run(&mut f).await;
        // The job was picked up and lost before, so it is on its last attempt
        // while the run has only tried once.
        sqlx::query("UPDATE jobs SET attempts = max_attempts - 1")
            .execute(&f.pool)
            .await
            .unwrap();

        let job = f
            .queue
            .dequeue_class(JobClass::Workflow)
            .await
            .unwrap()
            .unwrap();
        f.worker.handle_job(job).await;

        let run = workflows.find_run(run.id).await.unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.finished_at.is_some());
        assert!(run.last_error.is_some());
        assert_eq!(f.queue.pending_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_stuck_webhook_does_not_hold_up_document_processing() {
        let mut f = fixture("text").await;
        let (workflows, run) = queue_unreachable_run(&mut f).await;
        // A target that accepts connections and never answers
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        let mut stuck = workflows
            .find_by_id(crate::database::test_vault(), run.workflow_id)
            .await
            .unwrap()
            .unwrap();
        stuck.actions = vec![WorkflowAction::Webhook {
            url,
            method: Default::default(),
            headers: Default::default(),
            secret: None,
        }];
        sqlx::query("UPDATE workflow_runs SET actions = ?")
            .bind(serde_json::to_string(&stuck.actions).unwrap())
            .execute(&f.pool)
            .await
            .unwrap();
        f.queue
            .enqueue(JobType::process_document(f.doc_id))
            .await
            .unwrap();

        let (_stop, shutdown) = watch::channel(false);
        let processed = async {
            loop {
                let doc = f
                    .repo
                    .find_by_id(crate::database::test_vault(), f.doc_id)
                    .await
                    .unwrap()
                    .unwrap();
                if doc.ocr_status == ProcessingStatus::Done {
                    return;
                }
                sleep(Duration::from_millis(50)).await;
            }
        };
        tokio::select! {
            _ = f.worker.run(shutdown) => panic!("the worker stopped on its own"),
            done = tokio::time::timeout(Duration::from_secs(5), processed) => {
                done.expect("OCR finished while the webhook was still hanging");
            }
        }

        let run = workflows.find_run(run.id).await.unwrap().unwrap();
        assert_eq!(
            run.status,
            RunStatus::Pending,
            "the webhook is still in flight"
        );
    }

    /// Records each span as it starts. A job span can outlive the test: its
    /// sqlx children close on a pool thread that never reports to the test
    /// subscriber (#395), so the job span may never end and be exported.
    #[derive(Clone, Debug, Default)]
    struct StartedSpans(std::sync::Arc<std::sync::Mutex<Vec<opentelemetry_sdk::trace::SpanData>>>);

    impl opentelemetry_sdk::trace::SpanProcessor for StartedSpans {
        fn on_start(&self, span: &mut opentelemetry_sdk::trace::Span, _: &opentelemetry::Context) {
            self.0.lock().unwrap().extend(span.exported_data());
        }

        fn on_end(&self, _: opentelemetry_sdk::trace::SpanData) {}

        fn force_flush(&self) -> opentelemetry_sdk::error::OTelSdkResult {
            Ok(())
        }

        fn shutdown_with_timeout(&self, _: Duration) -> opentelemetry_sdk::error::OTelSdkResult {
            Ok(())
        }
    }

    /// A dispatcher that stays registered for the whole test binary. While a
    /// single dispatcher is registered, tracing-core caches a callsite's
    /// interest from whichever thread hits it first, so a test without a
    /// subscriber could disable the job span for the traced test. Two
    /// registered dispatchers make every rebuild ask the traced test's one.
    static ANCHOR: std::sync::LazyLock<tracing::Dispatch> = std::sync::LazyLock::new(|| {
        tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default())
    });

    /// Run an `extract_text` job queued under a remote parent, after `adjust`
    /// changes its row; return the job span as it started.
    async fn traced_job_span(adjust: &str) -> opentelemetry_sdk::trace::SpanData {
        use opentelemetry::propagation::TextMapPropagator;
        use opentelemetry::trace::TracerProvider as _;
        use tracing_subscriber::layer::SubscriberExt;

        std::sync::LazyLock::force(&ANCHOR);
        let started = StartedSpans::default();
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_span_processor(started.clone())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("folio-test")));
        let _default = tracing::subscriber::set_default(subscriber);

        let f = fixture("OCR").await;
        let request = info_span!("request");
        let headers = std::collections::HashMap::from([(
            "traceparent".to_string(),
            format!("00-{TRACED}-1234567812345678-01"),
        )]);
        let _ = request.set_parent(
            opentelemetry_sdk::propagation::TraceContextPropagator::new().extract(&headers),
        );
        f.queue
            .enqueue(JobType::extract_text(f.doc_id))
            .instrument(request)
            .await
            .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(adjust.to_string()))
            .execute(&f.pool)
            .await
            .unwrap();
        let job = f.queue.dequeue().await.unwrap().unwrap();
        f.worker.handle_job(job).await;
        let spans = started.0.lock().unwrap();
        spans
            .iter()
            .find(|span| span.name == "job extract_text")
            .cloned()
            .expect("a job span")
    }

    const TRACED: &str = "11223344556677881122334455667788";

    #[tokio::test]
    async fn a_first_attempt_continues_the_trace_that_queued_it() {
        let span = traced_job_span("SELECT 1").await;
        assert_eq!(span.span_context.trace_id().to_string(), TRACED);
        assert!(span.links.is_empty());
    }

    #[tokio::test]
    async fn a_retry_starts_a_new_trace_linked_to_the_one_that_queued_it() {
        let span = traced_job_span("UPDATE jobs SET attempts = 1").await;
        assert_ne!(span.span_context.trace_id().to_string(), TRACED);
        assert_eq!(span.parent_span_id, opentelemetry::trace::SpanId::INVALID);
        let links: Vec<_> = span
            .links
            .iter()
            .map(|link| link.span_context.trace_id().to_string())
            .collect();
        assert_eq!(links, [TRACED]);
    }

    #[tokio::test]
    async fn a_delayed_job_starts_a_new_trace_linked_to_the_one_that_queued_it() {
        let span = traced_job_span(
            "UPDATE jobs SET created_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hour')",
        )
        .await;
        assert_ne!(span.span_context.trace_id().to_string(), TRACED);
        assert_eq!(span.parent_span_id, opentelemetry::trace::SpanId::INVALID);
        let links: Vec<_> = span
            .links
            .iter()
            .map(|link| link.span_context.trace_id().to_string())
            .collect();
        assert_eq!(links, [TRACED]);
    }
}
