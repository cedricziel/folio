//! Folio Application - Main entry point

mod healthcheck;

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::watch;
use tracing::info;
use tracing_subscriber::FmtSubscriber;

use folio_api::{create_router, AppState, IngestionState};
use folio_core::services::{
    EventingDocumentRepository, IngestPipeline, IngestScheduler, WorkflowEngine,
};
use folio_core::{ports::HealthCheck, EventBroadcaster};
use folio_infra::{
    create_pool, run_migrations, Config, DatabaseHealthCheck, DynamicEmbeddingService,
    DynamicOcrService, FileStorage, JobQueue, SourceFactory, SqliteCorrespondentRepository,
    SqliteDocumentRepository, SqliteDocumentTypeRepository, SqliteIngestLedger,
    SqliteSearchService, SqliteSettingsRepository, SqliteSourceRepository, SqliteTagRepository,
    SqliteWorkflowRepository, StorageDocumentImporter, StorageFileMetadata, ThumbnailService,
    WebhookClient, Worker, WorkerConfig, WorkflowRunner,
};

#[tokio::main]
async fn main() -> Result<()> {
    // `healthcheck` needs only the port, so it skips logging, migrations and the worker
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        let config = Config::load().context("Failed to load configuration")?;
        return match healthcheck::run(config.server.port).await {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("healthcheck failed: {e:#}");
                std::process::exit(1);
            }
        };
    }

    // Initialize logging
    FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("Starting Folio...");

    // Load configuration
    let config = Config::load().context("Failed to load configuration")?;

    info!("Configuration loaded");
    info!("  Server: {}:{}", config.server.host, config.server.port);
    info!("  Database: {}", config.database.url);
    info!("  Storage: {}", config.storage.path);
    info!("  AI Provider: {}", config.ai.provider);

    // Initialize database
    let pool = create_pool(&config.database.url)
        .await
        .context("Failed to connect to database")?;

    info!("Connected to database");

    // Run migrations
    run_migrations(&pool)
        .await
        .context("Failed to run database migrations")?;

    info!("Database migrations complete");

    // Initialize storage
    let storage = FileStorage::new(&config.storage.path);
    storage
        .init()
        .await
        .context("Failed to initialize storage")?;

    // Initialize thumbnail service
    let thumbnail_service = Arc::new(ThumbnailService::new(&config.storage.path));
    thumbnail_service
        .init()
        .await
        .context("Failed to initialize thumbnail service")?;

    info!("Storage initialized at: {}", config.storage.path);

    // Create repositories
    let document_repo = Arc::new(SqliteDocumentRepository::new(pool.clone()));
    let tag_repo = Arc::new(SqliteTagRepository::new(pool.clone()));
    let correspondent_repo = Arc::new(SqliteCorrespondentRepository::new(pool.clone()));
    let document_type_repo = Arc::new(SqliteDocumentTypeRepository::new(pool.clone()));
    let settings_repo = Arc::new(SqliteSettingsRepository::new(pool.clone()));
    let workflow_repo = Arc::new(SqliteWorkflowRepository::new(pool.clone()));

    // Create AI services (dynamic - reads from database settings with config.toml fallback)
    let ocr_service = Arc::new(DynamicOcrService::new(
        settings_repo.clone(),
        config.ai.clone(),
        &config.storage.path,
    ));
    let embedding_service = Arc::new(DynamicEmbeddingService::new(
        settings_repo.clone(),
        config.ai.clone(),
    ));
    let search_service = Arc::new(SqliteSearchService::new(pool.clone()));

    // Create job queue
    let job_queue = JobQueue::new(pool.clone());

    // Create event broadcaster for SSE
    let event_broadcaster = EventBroadcaster::new(100);

    // The HTTP layer gets a repository that announces creates, updates, and
    // deletes; the worker and workflow actions keep the plain one so they
    // control their own events.
    let api_document_repo = Arc::new(EventingDocumentRepository::new(
        document_repo.clone(),
        event_broadcaster.clone(),
    ));

    let webhook_client = WebhookClient::new();
    let public_url = config.server.base_url();
    let workflow_runner = Arc::new(WorkflowRunner::new(
        workflow_repo.clone(),
        document_repo.clone(),
        webhook_client.clone(),
    ));
    let workflow_engine = Arc::new(WorkflowEngine::new(
        workflow_repo.clone(),
        document_repo.clone(),
        correspondent_repo.clone(),
        document_type_repo.clone(),
        Arc::new(job_queue.clone()),
        Arc::new(StorageFileMetadata::new(FileStorage::new(
            &config.storage.path,
        ))),
        public_url.clone(),
    ));

    let ledger = Arc::new(SqliteIngestLedger::new(pool.clone()));
    let source_repo = Arc::new(SqliteSourceRepository::new(pool.clone()));
    let source_factory = Arc::new(SourceFactory::new(ledger.clone()));
    let configured_sources = config.ingest.definitions();
    let ingest_pipeline = Arc::new(IngestPipeline::new(
        document_repo.clone(),
        tag_repo.clone(),
        workflow_repo.clone(),
        ledger,
        Arc::new(StorageDocumentImporter::new(
            api_document_repo.clone(),
            FileStorage::new(&config.storage.path),
            job_queue.clone(),
        )),
    ));
    let ingest_scheduler = IngestScheduler::new(
        source_repo.clone(),
        configured_sources.clone(),
        source_factory.clone(),
        ingest_pipeline,
    );

    info!("Services initialized");

    // Create shutdown channel for worker
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Start background worker
    let worker = Worker::new(
        job_queue.clone(),
        document_repo.clone(),
        ocr_service.clone(),
        embedding_service.clone(),
        search_service.clone(),
        thumbnail_service.clone(),
        &config.storage.path,
        WorkerConfig::default(),
        event_broadcaster.clone(),
    )
    .with_workflow_runner(workflow_runner);

    let engine_handle =
        tokio::spawn(workflow_engine.run(event_broadcaster.subscribe(), shutdown_tx.subscribe()));

    let scheduler_handle = tokio::spawn(ingest_scheduler.run(shutdown_tx.subscribe()));

    let worker_handle = tokio::spawn(async move {
        worker.run(shutdown_rx).await;
    });

    info!("Background job worker started");

    // Create application state
    let health_checks: Vec<Arc<dyn HealthCheck>> =
        vec![Arc::new(DatabaseHealthCheck::new(pool.clone()))];

    let state = AppState::new(
        api_document_repo,
        tag_repo,
        correspondent_repo,
        document_type_repo,
        settings_repo,
        ocr_service,
        embedding_service,
        search_service,
        thumbnail_service,
        job_queue,
        config.storage.path.clone(),
        event_broadcaster,
        workflow_repo,
        webhook_client,
        public_url,
        IngestionState {
            managed: source_repo,
            configured: Arc::new(configured_sources),
            factory: source_factory,
        },
    )
    .with_health_checks(health_checks);

    // Create router
    let app = create_router(state);

    // Start server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context(format!("Failed to bind to {}", addr))?;

    info!("🚀 Folio is running at http://{}", addr);

    // Run server with graceful shutdown
    let server = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal());

    if let Err(e) = server.await {
        tracing::error!("Server error: {}", e);
    }

    // Signal worker to shut down
    info!("Shutting down background worker...");
    let _ = shutdown_tx.send(true);

    // Wait for worker to finish (with timeout)
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), worker_handle).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), engine_handle).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), scheduler_handle).await;

    info!("Folio shutdown complete");
    Ok(())
}

/// Shutdown signal handler for graceful termination
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received");
}
