//! Background job worker

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::sleep;
use tracing::{debug, error, info, instrument};

use folio_core::ports::{DocumentRepository, EmbeddingService, OcrService, SearchService};

use super::{JobQueue, JobType};
use crate::storage::FileStorage;
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
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1000,
            stuck_check_interval_secs: 60,
            stuck_threshold_mins: 5,
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
    storage: FileStorage,
    config: WorkerConfig,
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
        storage_path: &str,
        config: WorkerConfig,
    ) -> Self {
        Self {
            queue,
            document_repo,
            ocr_service,
            embedding_service,
            search_service,
            thumbnail_service,
            storage: FileStorage::new(storage_path),
            config,
        }
    }

    /// Start the worker (runs until shutdown signal)
    pub async fn run(&self, mut shutdown: watch::Receiver<bool>) {
        info!("Job worker started");

        let mut stuck_check_counter = 0u64;

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

            // Try to get a job
            match self.queue.dequeue().await {
                Ok(Some(job)) => {
                    debug!("Processing job {}: {:?}", job.id, job.job_type.type_name());

                    let result = self.process_job(&job.job_type).await;

                    match result {
                        Ok(()) => {
                            if let Err(e) = self.queue.complete(job.id).await {
                                error!("Failed to mark job {} as complete: {}", job.id, e);
                            }
                        }
                        Err(e) => {
                            let error_msg = e.to_string();
                            if let Err(e) = self.queue.fail(job.id, &error_msg, true).await {
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

    /// Process a single job
    #[instrument(skip(self))]
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
        let data = self.storage.read(file_path).await?;

        let mime_type = document
            .mime_type
            .as_deref()
            .unwrap_or("application/octet-stream");

        self.thumbnail_service
            .generate(document_id, &data, mime_type, document.content.as_deref())
            .await?;

        info!("Generated thumbnail for document {}", document_id);
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
        let data = self.storage.read(file_path).await?;

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
        let document = self
            .document_repo
            .find_by_id(document_id)
            .await?
            .ok_or("Document not found")?;

        let file_path = match &document.file_path {
            Some(p) => p.clone(),
            None => return Err("No file path".into()),
        };

        let data = self.storage.read(&file_path).await?;
        let mime_type = document
            .mime_type
            .as_deref()
            .unwrap_or("application/octet-stream");

        // Step 1: Generate thumbnail
        if let Err(e) = self
            .thumbnail_service
            .generate(document_id, &data, mime_type, document.content.as_deref())
            .await
        {
            debug!("Thumbnail generation failed: {}", e);
        }

        // Step 2: Extract text if needed
        let content = if document.content.is_none() {
            match self.ocr_service.extract_text(&data, mime_type).await {
                Ok(text) if !text.is_empty() => {
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
                    Some(text)
                }
                Ok(_) => None,
                Err(e) => {
                    debug!("OCR failed: {}", e);
                    None
                }
            }
        } else {
            document.content.clone()
        };

        // Step 3: Generate embedding if we have content
        if let Some(text) = content {
            if !text.is_empty() {
                match self.embedding_service.generate_embedding(&text).await {
                    Ok(embedding) => {
                        self.search_service
                            .store_embedding(document_id, &embedding)
                            .await?;
                    }
                    Err(e) => {
                        debug!("Embedding generation failed: {}", e);
                    }
                }
            }
        }

        info!("Processed document {}", document_id);
        Ok(())
    }
}
