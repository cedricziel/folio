//! Application state

use std::sync::Arc;

use folio_core::error::Result;
use folio_core::ports::SearchResult;
use folio_core::ports::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, EmbeddingService,
    OcrService, SearchService, SettingsRepository, TagRepository,
};
use folio_infra::{JobQueue, PipelineContext, ThumbnailService};

use folio_core::EventBroadcaster;

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
        }
    }
}

impl AppState {
    /// Semantic search over document chunks embedded with the active model
    pub async fn semantic_search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let batch = self.embedding_service.embed(&[query]).await?;
        let Some(vector) = batch.vectors.first() else {
            return Ok(Vec::new());
        };
        self.search_service
            .search(&batch.model, vector, limit)
            .await
    }
}
