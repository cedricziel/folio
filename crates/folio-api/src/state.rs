//! Application state

use std::sync::Arc;

use folio_core::ports::{
    CorrespondentRepository, DocumentRepository, DocumentStore, DocumentTypeRepository,
    EmbeddingService, HealthCheck, OcrService, SearchService, SettingsRepository, TagRepository,
};
use folio_infra::{JobQueue, ThumbnailService};

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
    pub thumbnail_service: Arc<ThumbnailService>,
    pub job_queue: JobQueue,
    pub store: Arc<dyn DocumentStore>,
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
        thumbnail_service: Arc<ThumbnailService>,
        job_queue: JobQueue,
        store: Arc<dyn DocumentStore>,
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
            thumbnail_service,
            job_queue,
            store,
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
