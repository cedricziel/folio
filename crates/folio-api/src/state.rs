//! Application state

use std::sync::Arc;

use folio_core::error::Result;
use folio_core::ports::SearchResult;
use folio_core::ports::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, EmbeddingService,
    HealthCheck, OcrService, SearchService, SettingsRepository, TagRepository,
};
use folio_infra::{JobQueue, PipelineContext, ThumbnailService};

use folio_core::EventBroadcaster;

use crate::routes::HealthChecks;

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub document_repo: Arc<dyn DocumentRepository>,
    pub tag_repo: Arc<dyn TagRepository>,
    pub correspondent_repo: Arc<dyn CorrespondentRepository>,
    pub document_type_repo: Arc<dyn DocumentTypeRepository>,
    pub settings_repo: Arc<dyn SettingsRepository>,
    pub ocr_service: Arc<dyn OcrService>,
    pub embedding_service: Arc<dyn EmbeddingService>,
    pub search_service: Arc<dyn SearchService>,
    pub pipeline: Arc<PipelineContext>,
    pub thumbnail_service: Arc<ThumbnailService>,
    pub job_queue: JobQueue,
    pub storage_path: String,
    pub event_broadcaster: EventBroadcaster,
    pub health_checks: HealthChecks,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        document_repo: Arc<dyn DocumentRepository>,
        tag_repo: Arc<dyn TagRepository>,
        correspondent_repo: Arc<dyn CorrespondentRepository>,
        document_type_repo: Arc<dyn DocumentTypeRepository>,
        settings_repo: Arc<dyn SettingsRepository>,
        ocr_service: Arc<dyn OcrService>,
        embedding_service: Arc<dyn EmbeddingService>,
        search_service: Arc<dyn SearchService>,
        pipeline: Arc<PipelineContext>,
        thumbnail_service: Arc<ThumbnailService>,
        job_queue: JobQueue,
        storage_path: String,
        event_broadcaster: EventBroadcaster,
    ) -> Self {
        Self {
            document_repo,
            tag_repo,
            correspondent_repo,
            document_type_repo,
            settings_repo,
            ocr_service,
            embedding_service,
            search_service,
            pipeline,
            thumbnail_service,
            job_queue,
            storage_path,
            event_broadcaster,
            health_checks: Arc::from(Vec::new()),
        }
    }

    /// Register the readiness checks reported by `/readyz`.
    pub fn with_health_checks(mut self, checks: Vec<Arc<dyn HealthCheck>>) -> Self {
        self.health_checks = checks.into();
        self
    }
}

impl AppState {
    /// Search documents by meaning, falling back to title and text matching
    ///
    /// The fallback applies when embeddings are disabled or fail, and when no
    /// stored vector matches (for example before the first document is embedded).
    pub async fn search_documents(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        match self.semantic_search(query, limit).await {
            Ok(results) if !results.is_empty() => return Ok(results),
            Ok(_) => {}
            Err(e) => tracing::debug!("Semantic search unavailable, using text match: {}", e),
        }
        self.text_search(query, limit).await
    }

    async fn semantic_search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let batch = self.embedding_service.embed(&[query]).await?;
        let Some(vector) = batch.vectors.first() else {
            return Ok(Vec::new());
        };
        self.search_service
            .search(&batch.model, vector, limit)
            .await
    }

    async fn text_search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let needle = query.to_lowercase();
        let documents = self.document_repo.list(None).await?;
        Ok(documents
            .into_iter()
            .filter(|d| {
                d.title.to_lowercase().contains(&needle)
                    || d.content
                        .as_deref()
                        .is_some_and(|c| c.to_lowercase().contains(&needle))
            })
            .take(limit)
            .map(|d| SearchResult {
                document_id: d.id,
                score: 1.0,
                snippet: None,
            })
            .collect())
    }
}
