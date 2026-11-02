//! Background job worker

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::watch;
use tokio::time::sleep;
use tracing::{debug, error, info, info_span, warn, Instrument};

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
    async fn emit(&self, vault: VaultId, event: DocumentEvent) -> Result<(), BoxError> {
        self.outbox
            .append_document(vault, &event, EventOrigin::User)
            .await?;
        Ok(())
    }

    /// Like [`emit`](Self::emit) for a progress hint that must not fail the step it follows
    async fn emit_logged(&self, vault: VaultId, event: DocumentEvent) {
        if let Err(e) = self.emit(vault, event).await {
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
        let started = Instant::now();
        let Some(result) = run_with_lease(
            &self.queue,
            &job,
            self.process_job(&job.job_type).instrument(span.clone()),
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
                    .fail(&job, &error_msg, true)
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
                self.queue
                    .enqueue(JobType::PurgeDocument {
                        document_id: document.id,
                    })
                    .await?;
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
                .enqueue(JobType::ProcessDocument {
                    document_id: doc.id,
                })
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
                    .enqueue(JobType::GenerateEmbedding {
                        document_id: doc.id,
                    })
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

    /// Process a single job, then clean up after a document that was deleted while it ran
    async fn process_job(&self, job_type: &JobType) -> Result<(), BoxError> {
        if let Some(document_id) = job_type.document_id() {
            if !matches!(job_type, JobType::PurgeDocument { .. })
                && self.document_repo.locate(document_id).await?.is_none()
            {
                self.purge_if_deleted(document_id).await?;
                return Ok(());
            }
        }
        let result = self.run_job(job_type).await;
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
            JobType::ExtractText { document_id } => self.process_ocr(*document_id).await,
            JobType::GenerateEmbedding { document_id } => {
                self.process_embedding(*document_id).await
            }
            JobType::ProcessDocument { document_id } => self.process_document(*document_id).await,
            JobType::ExecuteWorkflowRun { run_id } => self.execute_workflow_run(*run_id).await,
            JobType::PurgeDocument { document_id } => self.purge_requested(*document_id).await,
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
        self.emit(vault, DocumentEvent::ThumbnailReady { document_id })
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

    /// Process OCR text extraction
    async fn process_ocr(&self, document_id: uuid::Uuid) -> Result<(), BoxError> {
        let vault = self.vault_of(document_id).await?;
        let document = self
            .document_repo
            .find_by_id(vault, document_id)
            .await?
            .ok_or("Document not found")?;

        let set_status = |status| {
            self.document_repo
                .set_stage_status(vault, document_id, ProcessingStage::Ocr, status)
        };

        if document.content.as_deref().is_some_and(|c| !c.is_empty()) {
            debug!("Document {} already has content, skipping OCR", document_id);
            set_status(ProcessingStatus::Done).await?;
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
        if has_text {
            self.emit(vault, DocumentEvent::OcrComplete { document_id })
                .await?;
        }
        self.schedule_embedding(vault, document_id, has_text).await
    }

    /// End the embedding stage, the last step of the pipeline, and announce that processing finished
    async fn finish_embed_stage(
        &self,
        vault: VaultId,
        document_id: uuid::Uuid,
        status: ProcessingStatus,
    ) -> Result<(), BoxError> {
        self.document_repo
            .set_stage_status(vault, document_id, ProcessingStage::Embed, status)
            .await?;
        self.emit(vault, DocumentEvent::Processed { document_id })
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
                .enqueue(JobType::GenerateEmbedding { document_id })
                .await?;
        } else {
            self.finish_embed_stage(vault, document_id, ProcessingStatus::Skipped)
                .await?;
        }
        Ok(())
    }

    /// Process embedding generation
    async fn process_embedding(&self, document_id: uuid::Uuid) -> Result<(), BoxError> {
        let vault = self.vault_of(document_id).await?;
        let document = self
            .document_repo
            .find_by_id(vault, document_id)
            .await?
            .ok_or("Document not found")?;

        let content = document.content.unwrap_or_default();
        let chunks = chunk_text(&content, DEFAULT_CHUNK_CHARS, DEFAULT_OVERLAP_CHARS);
        if chunks.is_empty() {
            self.finish_embed_stage(vault, document_id, ProcessingStatus::Skipped)
                .await?;
            return Ok(());
        }

        if self.pipeline.settings().await.embeddings.provider == "none" {
            self.finish_embed_stage(vault, document_id, ProcessingStatus::Skipped)
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
            self.finish_embed_stage(vault, document_id, ProcessingStatus::Done)
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
    async fn process_document(&self, document_id: uuid::Uuid) -> Result<(), BoxError> {
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

            // Generate thumbnails for all pages (limit to first 50 for performance)
            let max_pages = count.min(50);
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
        if has_text || settings.processing.auto_ocr {
            self.document_repo
                .set_stage_status(
                    vault,
                    document_id,
                    ProcessingStage::Ocr,
                    ProcessingStatus::Pending,
                )
                .await?;
            self.queue
                .enqueue(JobType::ExtractText { document_id })
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

        assert!(f.worker.process_embedding(f.doc_id).await.is_err());

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

    async fn drain(f: &Fixture) {
        while let Some(job) = f.queue.dequeue().await.unwrap() {
            f.worker.process_job(&job.job_type).await.unwrap();
            f.queue.complete(&job).await.unwrap();
        }
    }

    #[tokio::test]
    async fn long_work_renews_its_lease_while_pending() {
        let f = fixture("OCR").await;
        f.queue
            .enqueue(JobType::ExtractText {
                document_id: f.doc_id,
            })
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
            .enqueue(JobType::ExtractText {
                document_id: uuid::Uuid::new_v4(),
            })
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
            .enqueue(JobType::ExtractText {
                document_id: uuid::Uuid::new_v4(),
            })
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
            .enqueue(JobType::ExtractText {
                document_id: f.doc_id,
            })
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
            .enqueue(JobType::ExtractText {
                document_id: f.doc_id,
            })
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
            .enqueue(JobType::ProcessDocument {
                document_id: f.doc_id,
            })
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
            .enqueue(JobType::ProcessDocument {
                document_id: f.doc_id,
            })
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
            .process_job(&JobType::ProcessDocument {
                document_id: f.doc_id,
            })
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
        f.worker.store = f.storage.clone();

        f.worker.handle_job(retry).await;

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
            .process_job(&JobType::ProcessDocument {
                document_id: f.doc_id,
            })
            .await
            .unwrap();
        f.worker
            .process_job(&JobType::PurgeDocument {
                document_id: f.doc_id,
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
            .enqueue(JobType::ProcessDocument {
                document_id: f.doc_id,
            })
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
                document_id: f.doc_id
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
            .process_job(&JobType::GenerateEmbedding {
                document_id: f.doc_id,
            })
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
            .enqueue(JobType::ProcessDocument {
                document_id: f.doc_id,
            })
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
            .process_job(&JobType::ProcessDocument {
                document_id: doc2.id,
            })
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
            .process_job(&JobType::ExtractText {
                document_id: f.doc_id
            })
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
            .enqueue(JobType::ProcessDocument {
                document_id: f.doc_id,
            })
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
}
