//! Application state

use std::sync::Arc;

use folio_core::ports::{
    CorrespondentRepository, DocumentRepository, DocumentTypeRepository, EmbeddingService,
    OcrService, SearchService, TagRepository,
};
use folio_infra::{JobQueue, ThumbnailService};

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub document_repo: Arc<dyn DocumentRepository>,
    pub tag_repo: Arc<dyn TagRepository>,
    pub correspondent_repo: Arc<dyn CorrespondentRepository>,
    pub document_type_repo: Arc<dyn DocumentTypeRepository>,
    pub ocr_service: Arc<dyn OcrService>,
    pub embedding_service: Arc<dyn EmbeddingService>,
    pub search_service: Arc<dyn SearchService>,
    pub thumbnail_service: Arc<ThumbnailService>,
    pub job_queue: JobQueue,
    pub storage_path: String,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        document_repo: Arc<dyn DocumentRepository>,
        tag_repo: Arc<dyn TagRepository>,
        correspondent_repo: Arc<dyn CorrespondentRepository>,
        document_type_repo: Arc<dyn DocumentTypeRepository>,
        ocr_service: Arc<dyn OcrService>,
        embedding_service: Arc<dyn EmbeddingService>,
        search_service: Arc<dyn SearchService>,
        thumbnail_service: Arc<ThumbnailService>,
        job_queue: JobQueue,
        storage_path: String,
    ) -> Self {
        Self {
            document_repo,
            tag_repo,
            correspondent_repo,
            document_type_repo,
            ocr_service,
            embedding_service,
            search_service,
            thumbnail_service,
            job_queue,
            storage_path,
        }
    }
}
