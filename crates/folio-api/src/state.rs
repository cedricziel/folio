//! Application state

use std::sync::Arc;

use folio_core::error::Result;
use folio_core::models::IngestionSourceDef;
use folio_core::models::VaultId;
use folio_core::ports::{
    CorrespondentRepository, DocumentEntryRepository, DocumentImporter, DocumentRepository,
    DocumentStore, DocumentTypeRepository, EmbeddingService, HealthCheck, IngestLedger,
    IngestionSourceFactory, OcrService, OutboxStore, SearchResult, SearchService,
    SettingsRepository, SourceRepository, TagRepository, WorkflowRepository,
};
use folio_core::services::{Importer, SearchServiceFacade, VaultService};
use folio_infra::{JobQueue, PipelineContext, SourceFactory, ThumbnailService, WebhookClient};

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
    pub entry_repo: Arc<dyn DocumentEntryRepository>,
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
    pub store: Arc<dyn DocumentStore>,
    pub event_broadcaster: EventBroadcaster,
    /// Source of truth for events; SSE replays from it
    pub outbox: Arc<dyn OutboxStore>,
    pub health_checks: HealthChecks,
    pub workflow_repo: Arc<dyn WorkflowRepository>,
    pub webhook_client: WebhookClient,
    /// Externally reachable base URL, used for links in webhook payloads
    pub public_url: String,
    /// The one path that stores a new document
    pub importer: Arc<dyn DocumentImporter>,
    pub ingestion: IngestionState,
    pub vaults: Arc<VaultService>,
    pub access: Arc<dyn folio_core::ports::AccessPolicy>,
    /// Whether `/mcp` is served while the `mcp.enabled` setting is unset
    pub mcp_default: bool,
    pub client_telemetry: Option<Arc<crate::client_telemetry::ClientTelemetry>>,
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
        entry_repo: Arc<dyn DocumentEntryRepository>,
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
        store: Arc<dyn DocumentStore>,
        event_broadcaster: EventBroadcaster,
        outbox: Arc<dyn OutboxStore>,
        workflow_repo: Arc<dyn WorkflowRepository>,
        webhook_client: WebhookClient,
        public_url: String,
        ingestion: IngestionState,
        vaults: Arc<VaultService>,
        access: Arc<dyn folio_core::ports::AccessPolicy>,
    ) -> Self {
        let importer = Arc::new(Importer::new(
            document_repo.clone(),
            store.clone(),
            Arc::new(job_queue.clone()),
        ));
        Self {
            document_repo,
            entry_repo,
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
            store,
            event_broadcaster,
            outbox,
            health_checks: Arc::from(Vec::new()),
            workflow_repo,
            webhook_client,
            public_url,
            importer,
            ingestion,
            vaults,
            access,
            mcp_default: false,
            client_telemetry: None,
        }
    }

    /// What /settings shows for `mcp.enabled` until an admin saves a choice.
    pub fn with_mcp_default(mut self, enabled: bool) -> Self {
        self.mcp_default = enabled;
        self
    }

    /// Register the readiness checks reported by `/readyz`.
    pub fn with_health_checks(mut self, checks: Vec<Arc<dyn HealthCheck>>) -> Self {
        self.health_checks = checks.into();
        self
    }
}

impl AppState {
    /// Search documents: literal title/text matches first, then semantic neighbours
    pub async fn search_documents(
        &self,
        vault: VaultId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let facade = SearchServiceFacade::new(
            self.document_repo.clone(),
            self.embedding_service.clone(),
            self.search_service.clone(),
        );
        Ok(facade
            .hybrid_search(vault, query, limit)
            .await?
            .into_iter()
            .map(|s| SearchResult {
                document_id: s.document.id,
                score: s.score,
                snippet: None,
            })
            .collect())
    }
}
