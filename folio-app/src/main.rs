//! Folio Application - Main entry point

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::watch;
use tracing::info;
use tracing_subscriber::FmtSubscriber;

use folio_api::{create_router, AppState};
use folio_infra::{
    create_pool, run_migrations, AiServiceFactory, Config, FileStorage, JobQueue,
    SqliteCorrespondentRepository, SqliteDocumentRepository, SqliteDocumentTypeRepository,
    SqliteSearchService, SqliteTagRepository, ThumbnailService, Worker, WorkerConfig,
};

#[tokio::main]
async fn main() -> Result<()> {
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

    // Create AI services
    let ocr_service = AiServiceFactory::create_ocr_service(&config.ai);
    let embedding_service = AiServiceFactory::create_embedding_service(&config.ai);
    let search_service = Arc::new(SqliteSearchService::new(pool.clone()));

    // Create job queue
    let job_queue = JobQueue::new(pool.clone());

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
    );

    let worker_handle = tokio::spawn(async move {
        worker.run(shutdown_rx).await;
    });

    info!("Background job worker started");

    // Create application state
    let state = AppState::new(
        document_repo,
        tag_repo,
        correspondent_repo,
        document_type_repo,
        ocr_service,
        embedding_service,
        search_service,
        thumbnail_service,
        job_queue,
        config.storage.path.clone(),
    );

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
