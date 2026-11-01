//! Folio Application - Main entry point

mod healthcheck;

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::info;
use tracing_subscriber::FmtSubscriber;

use folio_api::{create_router, AppState};
use folio_core::ports::{HealthCheck, SettingsRepository};
use folio_core::services::LibraryService;
use folio_core::EventBroadcaster;
use folio_infra::{
    create_pool, run_migrations, Config, DatabaseHealthCheck, DynamicEmbeddingService,
    DynamicOcrService, FileStorage, JobQueue, SqliteCorrespondentRepository,
    SqliteDocumentRepository, SqliteDocumentTypeRepository, SqliteSearchService,
    SqliteSettingsRepository, SqliteTagRepository, ThumbnailService, Worker, WorkerConfig,
};
use folio_mcp::McpOptions;

/// What the binary was asked to do
enum Mode {
    Server,
    McpStdio,
}

fn parse_args() -> Result<Mode> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => Ok(Mode::Server),
        ["mcp", "--stdio"] => Ok(Mode::McpStdio),
        _ => anyhow::bail!("usage: folio-app [mcp --stdio]"),
    }
}

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

    let mode = parse_args()?;

    // Initialize logging; stdout carries the protocol in stdio mode
    FmtSubscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
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

    let library = Arc::new(LibraryService::new(
        document_repo.clone(),
        tag_repo.clone(),
        correspondent_repo.clone(),
        document_type_repo.clone(),
        embedding_service.clone(),
        search_service.clone(),
        Arc::new(FileStorage::new(&config.storage.path)),
        Arc::new(job_queue.clone()),
    ));

    if matches!(mode, Mode::McpStdio) {
        let served = folio_mcp::serve_stdio(library).await;
        stop_worker(shutdown_tx, worker_handle).await;
        return served.map_err(|e| anyhow::anyhow!(e));
    }

    let mcp_settings: Arc<dyn SettingsRepository> = settings_repo.clone();
    if folio_mcp::is_enabled(mcp_settings.as_ref(), config.mcp.enabled).await {
        tracing::warn!(
            "MCP endpoint /mcp is enabled without authentication; \
             do not expose this server beyond localhost until an auth layer is configured"
        );
    }

    // Create application state
    let health_checks: Vec<Arc<dyn HealthCheck>> =
        vec![Arc::new(DatabaseHealthCheck::new(pool.clone()))];

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
    )
    .with_health_checks(health_checks);

    // Create router
    // The MCP endpoint is unauthenticated until an auth layer is applied here
    let mcp = folio_mcp::router(
        library,
        mcp_settings,
        McpOptions {
            enabled_by_default: config.mcp.enabled,
            allowed_hosts: config.mcp.allowed_hosts.clone(),
        },
    );
    let app = create_router(state).merge(mcp);

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

    stop_worker(shutdown_tx, worker_handle).await;

    info!("Folio shutdown complete");
    Ok(())
}

/// Signal the background worker to stop and wait for it, with a timeout
async fn stop_worker(shutdown_tx: watch::Sender<bool>, worker_handle: JoinHandle<()>) {
    info!("Shutting down background worker...");
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), worker_handle).await;
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
