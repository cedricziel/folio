//! Application state

use std::sync::Arc;

use folio_core::models::IngestionSourceDef;
use folio_core::ports::{
    CorrespondentRepository, DocumentImporter, DocumentRepository, DocumentTypeRepository,
    EmbeddingService, HealthCheck, IngestLedger, IngestionSourceFactory, OcrService, SearchService,
    SettingsRepository, SourceRepository, TagRepository, WorkflowRepository,
};
use folio_infra::{
    FileStorage, JobQueue, SourceFactory, StorageDocumentImporter, ThumbnailService, WebhookClient,
};

use folio_core::EventBroadcaster;

use crate::routes::HealthChecks;

/// What the UI needs to manage and test ingestion sources
#[derive(Clone)]
pub struct IngestionState {
    /// Sources created in the UI
    pub managed: Arc<dyn SourceRepository>,
    /// Sources from environment or `config.toml`, shown read-only
    pub configured: Arc<Vec<IngestionSourceDef>>,
    pub factory: Arc<dyn IngestionSourceFactory>,
}

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
    pub storage_path: String,
    pub event_broadcaster: EventBroadcaster,
    pub health_checks: HealthChecks,
    pub workflow_repo: Arc<dyn WorkflowRepository>,
    pub webhook_client: WebhookClient,
    /// Externally reachable base URL, used for links in webhook payloads
    pub public_url: String,
    /// The one path that stores a new document
    pub importer: Arc<dyn DocumentImporter>,
    pub ingestion: IngestionState,
}

impl IngestionState {
    pub fn new(
        managed: Arc<dyn SourceRepository>,
        ledger: Arc<dyn IngestLedger>,
        configured: Vec<IngestionSourceDef>,
    ) -> Self {
        Self {
            managed,
            configured: Arc::new(configured),
            factory: Arc::new(SourceFactory::new(ledger)),
        }
    }
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
        storage_path: String,
        event_broadcaster: EventBroadcaster,
        workflow_repo: Arc<dyn WorkflowRepository>,
        webhook_client: WebhookClient,
        public_url: String,
        ingestion: IngestionState,
    ) -> Self {
        let importer = Arc::new(StorageDocumentImporter::new(
            document_repo.clone(),
            FileStorage::new(&storage_path),
            job_queue.clone(),
        ));
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
            storage_path,
            event_broadcaster,
            health_checks: Arc::from(Vec::new()),
            workflow_repo,
            webhook_client,
            public_url,
            importer,
            ingestion,
        }
    }

    /// Register the readiness checks reported by `/readyz`.
    pub fn with_health_checks(mut self, checks: Vec<Arc<dyn HealthCheck>>) -> Self {
        self.health_checks = checks.into();
        self
    }
}
