//! Folio Application - Main entry point

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::watch;
use tracing::info;
use tracing_subscriber::FmtSubscriber;

use folio_api::{create_router, AppState, AuthState};
use folio_core::EventBroadcaster;
use folio_infra::{
    create_pool, run_migrations, Config, DynamicEmbeddingService, DynamicOcrService, FileStorage,
    JobQueue, SqliteAuthStore, SqliteCorrespondentRepository, SqliteDocumentRepository,
    SqliteDocumentTypeRepository, SqliteSearchService, SqliteSettingsRepository,
    SqliteTagRepository, ThumbnailService, Worker, WorkerConfig,
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
    let settings_repo = Arc::new(SqliteSettingsRepository::new(pool.clone()));

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
        settings_repo,
        ocr_service,
        embedding_service,
        search_service,
        thumbnail_service,
        job_queue,
        config.storage.path.clone(),
        event_broadcaster,
    );

    let auth_store = Arc::new(SqliteAuthStore::new(pool.clone()));
    let auth = AuthState::new(auth_store, config.server.public_url.clone())
        .with_required(config.auth.required);

    // Create router
    let app = create_router(state, auth);

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
