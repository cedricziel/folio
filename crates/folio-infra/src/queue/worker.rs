//! Background job worker

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tokio::time::sleep;
use tracing::{debug, error, info, info_span, Instrument};

use folio_core::events::{DocumentEvent, EventBroadcaster};
use folio_core::ports::{
    DocumentRepository, DocumentStore, EmbeddingService, OcrService, SearchService,
};

use super::{JobQueue, JobType};
use crate::telemetry::metrics;
use crate::thumbnail::ThumbnailService;

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
    thumbnail_service: Arc<ThumbnailService>,
    store: Arc<dyn DocumentStore>,
    config: WorkerConfig,
    event_broadcaster: EventBroadcaster,
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
            thumbnail_service,
            store,
            config,
            event_broadcaster,
        }
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
                        document.id = %job.job_type.document_id(),
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

    /// Find and re-queue orphaned documents (documents with no content and no active jobs)
    async fn repair_orphaned_documents(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Get all documents
        let documents = self.document_repo.list(None).await?;
        let mut repaired_count = 0;

        for doc in documents {
            // Skip documents that already have content
            if doc.content.is_some() {
                continue;
            }

            // Skip documents without a file path (nothing to process)
            if doc.file_path.is_none() {
                continue;
            }

            // Check if there's already an active job for this document
            let active_jobs = self.queue.find_active_jobs_for_document(doc.id).await?;

            if active_jobs.is_empty() {
                // No active jobs and no content - this document is orphaned, re-queue it
                info!("Repairing orphaned document {} ({})", doc.id, doc.title);
                self.queue
                    .enqueue(JobType::ProcessDocument {
                        document_id: doc.id,
                    })
                    .await?;
                repaired_count += 1;
            }
        }

        if repaired_count > 0 {
            info!("Repaired {} orphaned documents", repaired_count);
        }

        Ok(())
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
        }
    }

    /// Process thumbnail generation
    async fn process_thumbnail(
        &self,
        document_id: uuid::Uuid,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let document = self
            .document_repo
            .find_by_id(document_id)
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

    /// Process OCR text extraction
    async fn process_ocr(
        &self,
        document_id: uuid::Uuid,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let document = self
            .document_repo
            .find_by_id(document_id)
            .await?
            .ok_or("Document not found")?;

        // Skip if already has content
        if document.content.is_some() {
            debug!("Document {} already has content, skipping OCR", document_id);
            return Ok(());
        }

        // Read the file
        let file_path = document.file_path.as_ref().ok_or("No file path")?;
        let data = self.store.get_bytes(file_path).await?;

        let mime_type = document
            .mime_type
            .as_deref()
            .unwrap_or("application/octet-stream");

        // Extract text
        let content = self.ocr_service.extract_text(&data, mime_type).await?;

        if !content.is_empty() {
            // Update document with extracted content
            self.document_repo
                .update(
                    document_id,
                    folio_core::models::UpdateDocument {
                        content: Some(content.clone()),
                        ..Default::default()
                    },
                )
                .await?;

            info!(
                "Extracted text for document {} ({} chars)",
                document_id,
                content.len()
            );

            // Emit event
            self.event_broadcaster
                .send(DocumentEvent::OcrComplete { document_id });
        }

        Ok(())
    }

    /// Process embedding generation
    async fn process_embedding(
        &self,
        document_id: uuid::Uuid,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let document = self
            .document_repo
            .find_by_id(document_id)
            .await?
            .ok_or("Document not found")?;

        let content = document
            .content
            .as_ref()
            .ok_or("No content for embedding")?;

        if content.is_empty() {
            return Ok(());
        }

        let embedding = self.embedding_service.generate_embedding(content).await?;
        self.search_service
            .store_embedding(document_id, &embedding)
            .await?;

        info!("Generated embedding for document {}", document_id);
        Ok(())
    }

    /// Process a complete document (OCR, thumbnail, embedding)
    async fn process_document(
        &self,
        document_id: uuid::Uuid,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Starting document processing for {}", document_id);

        let document = self
            .document_repo
            .find_by_id(document_id)
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

        info!("Reading file from: {}", file_path);
        let data = self.store.get_bytes(&file_path).await?;
        info!("File read successfully, size: {} bytes", data.len());

        let mime_type = document
            .mime_type
            .as_deref()
            .unwrap_or("application/octet-stream");

        // Step 1: Generate thumbnail(s) and get page count for PDFs
        let (thumbnail_ok, page_count) = if mime_type == "application/pdf" {
            // Get page count
            let count = match self.thumbnail_service.get_pdf_page_count(&data).await {
                Ok(c) => c,
                Err(e) => {
                    debug!("Failed to get PDF page count: {}", e);
                    1
                }
            };

            // Store page count
            if let Err(e) = self.document_repo.set_page_count(document_id, count).await {
                debug!("Failed to set page count: {}", e);
            }

            // Generate thumbnails for all pages (limit to first 50 for performance)
            let max_pages = count.min(50);
            let mut success = false;

            for page in 1..=max_pages {
                match self
                    .thumbnail_service
                    .generate_page(document_id, &data, page)
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
                if let Err(e) = self.document_repo.set_page_count(document_id, 1).await {
                    debug!("Failed to set page count: {}", e);
                }
            }

            (ok, None)
        };

        // Log page count if available
        if let Some(count) = page_count {
            debug!("Document {} has {} pages", document_id, count);
        }

        // Step 2: Extract text if needed
        info!(
            "Step 2: OCR - document has existing content: {}",
            document.content.is_some()
        );
        let content = if document.content.is_none() {
            info!(
                "Calling OCR service for document {} (mime: {})",
                document_id, mime_type
            );
            match self.ocr_service.extract_text(&data, mime_type).await {
                Ok(text) if !text.is_empty() => {
                    info!(
                        "OCR successful for document {}, extracted {} characters",
                        document_id,
                        text.len()
                    );
                    // Update document with extracted content
                    self.document_repo
                        .update(
                            document_id,
                            folio_core::models::UpdateDocument {
                                content: Some(text.clone()),
                                ..Default::default()
                            },
                        )
                        .await?;

                    self.event_broadcaster
                        .send(DocumentEvent::OcrComplete { document_id });

                    Some(text)
                }
                Ok(_) => {
                    info!("OCR returned empty text for document {}", document_id);
                    None
                }
                Err(e) => {
                    error!("OCR failed for document {}: {}", document_id, e);
                    None
                }
            }
        } else {
            info!(
                "Skipping OCR - document {} already has content",
                document_id
            );
            document.content.clone()
        };

        // Step 3: Generate embedding if we have content
        info!(
            "Step 3: Embedding - have content: {}",
            content.as_ref().map(|c| c.len()).unwrap_or(0)
        );
        if let Some(text) = content {
            if !text.is_empty() {
                info!(
                    "Generating embedding for document {} ({} chars)",
                    document_id,
                    text.len()
                );
                match self.embedding_service.generate_embedding(&text).await {
                    Ok(embedding) => {
                        info!(
                            "Embedding generated for document {}, dimension: {}",
                            document_id,
                            embedding.len()
                        );
                        self.search_service
                            .store_embedding(document_id, &embedding)
                            .await?;
                        info!("Embedding stored for document {}", document_id);
                    }
                    Err(e) => {
                        error!(
                            "Embedding generation failed for document {}: {}",
                            document_id, e
                        );
                    }
                }
            }
        }

        info!("Completed processing document {}", document_id);

        // Emit final updated event if any processing happened
        if thumbnail_ok {
            self.event_broadcaster
                .send(DocumentEvent::Updated { document_id });
        }

        Ok(())
    }
}
