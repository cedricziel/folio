//! Background job worker

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tokio::time::sleep;
use tracing::{debug, error, info, info_span, Instrument};

use folio_core::events::{DocumentEvent, EventBroadcaster};
use folio_core::models::{ProcessingStage, ProcessingStatus, VaultId};
use folio_core::ports::{
    DocumentRepository, DocumentStore, EmbeddedChunk, EmbeddingService, OcrService, SearchService,
};

use super::{JobQueue, JobType};
use crate::ai::chunker::{chunk_text, DEFAULT_CHUNK_CHARS, DEFAULT_OVERLAP_CHARS};
use crate::ai::PipelineContext;
use crate::telemetry::metrics;
use crate::thumbnail::ThumbnailService;
use crate::workflow::{RunOutcome, WorkflowRunner};

/// Chunks sent to the embedding provider per request
const EMBED_BATCH_SIZE: usize = 16;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Configuration for the job worker
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// How often to poll for jobs (in milliseconds)
    pub poll_interval_ms: u64,
    /// How often to reset stuck jobs (in seconds)
    pub stuck_check_interval_secs: u64,
    /// Jobs processing for longer than this are considered stuck (in minutes)
    pub stuck_threshold_mins: i64,
    /// How often to check for orphaned documents (in seconds)
    pub repair_check_interval_secs: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1000,
            stuck_check_interval_secs: 60,
            stuck_threshold_mins: 5,
            repair_check_interval_secs: 300, // Every 5 minutes
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
    event_broadcaster: EventBroadcaster,
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
        event_broadcaster: EventBroadcaster,
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
            event_broadcaster,
            workflow_runner: None,
        }
    }

    /// Let this worker execute workflow runs
    pub fn with_workflow_runner(mut self, runner: Arc<WorkflowRunner>) -> Self {
        self.workflow_runner = Some(runner);
        self
    }

    /// Start the worker (runs until shutdown signal)
    pub async fn run(&self, mut shutdown: watch::Receiver<bool>) {
        info!("Job worker started");

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
                if let Err(e) = self
                    .queue
                    .reset_stuck(self.config.stuck_threshold_mins)
                    .await
                {
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
            match self.queue.dequeue().await {
                Ok(Some(job)) => {
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
                    let result = self
                        .process_job(&job.job_type)
                        .instrument(span.clone())
                        .await;
                    metrics().job_finished(type_name, result.is_ok(), started);

                    match result {
                        Ok(()) => {
                            if let Err(e) =
                                self.queue.complete(job.id).instrument(span.clone()).await
                            {
                                error!("Failed to mark job {} as complete: {}", job.id, e);
                            }
                        }
                        Err(e) => {
                            let error_msg = e.to_string();
                            if let Err(e) = self
                                .queue
                                .fail(job.id, &error_msg, true)
                                .instrument(span.clone())
                                .await
                            {
                                error!("Failed to mark job {} as failed: {}", job.id, e);
                            }
                        }
                    }
                }
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

    /// Re-queue documents whose OCR or embedding stage is still pending but has no active job
    ///
    /// Driven by the per-stage status, never by whether `content` is set, so a
    /// document with legitimately empty OCR output is not picked up again.
    async fn repair_orphaned_documents(&self) -> Result<(), BoxError> {
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

    /// Process a single job
    async fn process_job(
        &self,
        job_type: &JobType,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
        }
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

        self.thumbnail_service
            .generate(document_id, &data, mime_type, document.content.as_deref())
            .await?;

        info!("Generated thumbnail for document {}", document_id);

        // Emit event
        self.event_broadcaster
            .send(DocumentEvent::ThumbnailReady { document_id });

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
            self.event_broadcaster
                .send(DocumentEvent::OcrComplete { document_id });
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
        self.event_broadcaster
            .send(DocumentEvent::Processed { document_id });
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
                model = batch.model;
                for (i, (text, embedding)) in batch_texts.iter().zip(batch.vectors).enumerate() {
                    embedded.push(EmbeddedChunk {
                        index: (batch_no * EMBED_BATCH_SIZE + i) as u32,
                        text: text.clone(),
                        embedding,
                    });
                }
            }

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
                match self
                    .thumbnail_service
                    .generate_page(document_id, local_pdf.path(), page)
                    .await
                {
                    Ok(_) => {
                        if page == 1 {
                            self.event_broadcaster
                                .send(DocumentEvent::ThumbnailReady { document_id });
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
            let ok = match self
                .thumbnail_service
                .generate(document_id, &data, mime_type, document.content.as_deref())
                .await
            {
                Ok(_) => {
                    self.event_broadcaster
                        .send(DocumentEvent::ThumbnailReady { document_id });
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
    use crate::database::{test_pool, SqliteDocumentRepository, SqliteSettingsRepository};
    use crate::SqliteSearchService;
    use async_trait::async_trait;
    use folio_core::error::Result;
    use folio_core::models::CreateDocument;
    use folio_core::ports::{EmbeddingBatch, OcrOutput, SettingsRepository};

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

    struct Fixture {
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
            .create(CreateDocument {
                title: "scan".into(),
                content: None,
                original_filename: Some("scan.png".into()),
                mime_type: Some("image/png".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let path = folio_core::ports::keys::original(doc.id);
        storage
            .put(&path, b"not a real png".as_slice().into())
            .await
            .unwrap();
        repo.set_file_path(doc.id, &path).await.unwrap();

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
            EventBroadcaster::new(16),
        );

        Fixture {
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
            f.queue.complete(job.id).await.unwrap();
        }
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

        let doc = f.repo.find_by_id(f.doc_id).await.unwrap().unwrap();
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

        let doc = f.repo.find_by_id(f.doc_id).await.unwrap().unwrap();
        assert_eq!(
            doc.content.as_deref(),
            Some("Invoice from ACME for services")
        );
        assert_eq!(doc.ocr_status, ProcessingStatus::Done);
        assert_eq!(doc.embed_status, ProcessingStatus::Done);

        let hits = f.search.search("test:model", &[1.0, 0.0], 5).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(f
            .search
            .search("other:model", &[1.0, 0.0], 5)
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

        let doc = f.repo.find_by_id(f.doc_id).await.unwrap().unwrap();
        assert_eq!(doc.ocr_status, ProcessingStatus::Done);
        assert_eq!(doc.embed_status, ProcessingStatus::Skipped);

        f.settings
            .set("processing.auto_ocr", "false")
            .await
            .unwrap();
        f.pipeline.invalidate().await;
        let doc2 = f
            .repo
            .create(CreateDocument {
                title: "second".into(),
                content: None,
                original_filename: None,
                mime_type: Some("image/png".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let path = folio_core::ports::keys::original(doc2.id);
        f.storage.put(&path, b"x".as_slice().into()).await.unwrap();
        f.repo.set_file_path(doc2.id, &path).await.unwrap();
        f.worker
            .process_job(&JobType::ProcessDocument {
                document_id: doc2.id,
            })
            .await
            .unwrap();
        let doc2 = f.repo.find_by_id(doc2.id).await.unwrap().unwrap();
        assert_eq!(doc2.ocr_status, ProcessingStatus::Skipped);
    }

    #[tokio::test]
    async fn failed_ocr_is_recorded_and_not_requeued_by_repair() {
        let f = fixture("x").await;
        f.repo
            .set_file_path(f.doc_id, "missing/file.png")
            .await
            .unwrap();

        assert!(f
            .worker
            .process_job(&JobType::ExtractText {
                document_id: f.doc_id
            })
            .await
            .is_err());

        let doc = f.repo.find_by_id(f.doc_id).await.unwrap().unwrap();
        assert_eq!(doc.ocr_status, ProcessingStatus::Failed);
        f.worker.repair_orphaned_documents().await.unwrap();
        assert_eq!(f.queue.pending_count().await.unwrap(), 0);
    }
}
