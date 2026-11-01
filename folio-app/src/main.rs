//! Folio Application - Main entry point

mod healthcheck;

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::info;

use folio_api::{create_router, AppState, AuthState};
use folio_core::ports::{HealthCheck, SettingsRepository};
use folio_core::services::LibraryService;
use folio_core::EventBroadcaster;
use folio_infra::{
    build_store, cleanup_legacy_layout, migrate_legacy_layout, telemetry, Config, Database,
    DatabaseHealthCheck, DynamicEmbeddingService, DynamicOcrService, ThumbnailService, Worker,
    WorkerConfig,
};
use folio_mcp::McpOptions;

/// What the binary was asked to do
enum Mode {
    Server,
    McpStdio,
    /// Delete old-layout copies left behind by the storage migration
    CleanupLegacyStorage,
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
        ["storage", "cleanup-legacy"] => Ok(Mode::CleanupLegacyStorage),
        _ => anyhow::bail!("usage: folio-app [mcp --stdio | storage cleanup-legacy]"),
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

    // Initialize logging (stderr: stdout carries the protocol in stdio mode)
    // and, when OTEL_* variables ask for it, OTLP export
    let telemetry = telemetry::init();
    if telemetry.is_enabled() {
        info!("OpenTelemetry export enabled");
    }

    info!("Starting Folio...");

    // Load configuration
    let config = Config::load().context("Failed to load configuration")?;

    info!("Configuration loaded");
    info!("  Server: {}:{}", config.server.host, config.server.port);
    info!("  Database: {}", redact_url(&config.database.url));
    info!("  Storage: {:?}", config.storage.backend);
    info!("  AI Provider: {}", config.ai.provider);

    // Initialize database
    let database = Database::connect(&config.database.url)
        .await
        .context("Failed to connect to database")?;

    info!("Connected to {:?} database", database.backend());

    database
        .migrate()
        .await
        .context("Failed to run database migrations")?;

    info!("Database migrations complete");

    // Initialize storage
    let store = build_store(&config.storage)
        .await
        .context("Failed to initialize storage")?;
    let thumbnail_service = Arc::new(ThumbnailService::new(store.clone()));

    info!("Storage initialized ({} backend)", store.backend());

    // Create repositories
    let repos = database.repositories().await;
    let document_repo = repos.documents;
    let tag_repo = repos.tags;
    let correspondent_repo = repos.correspondents;
    let document_type_repo = repos.document_types;
    let settings_repo = repos.settings;
    let auth_store = repos.auth;

    if matches!(mode, Mode::CleanupLegacyStorage) {
        let cleaned = cleanup_legacy_layout(store.as_ref(), document_repo.as_ref())
            .await
            .context("Failed to clean up old storage layout")?;
        info!("Removed old-layout files of {cleaned} documents");
        return Ok(());
    }

    migrate_legacy_layout(
        store.as_ref(),
        document_repo.as_ref(),
        settings_repo.as_ref(),
    )
    .await
    .context("Failed to migrate stored files to object keys")?;

    let stored_documents = document_repo.list(None).await.map(|d| d.len()).unwrap_or(0);
    telemetry::metrics().documents_changed(stored_documents as i64);

    // Create AI services (dynamic - reads from database settings with config.toml fallback)
    let ocr_service = Arc::new(DynamicOcrService::new(
        settings_repo.clone(),
        config.ai.clone(),
    ));
    let embedding_service = Arc::new(DynamicEmbeddingService::new(
        settings_repo.clone(),
        config.ai.clone(),
    ));
    let search_service = repos.search;

    // Create job queue
    let job_queue = repos.jobs;

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
        store.clone(),
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
        store.clone(),
        Arc::new(job_queue.clone()),
    ));

    if matches!(mode, Mode::McpStdio) {
        let served = folio_mcp::serve_stdio(library).await;
        stop_worker(shutdown_tx, worker_handle).await;
        return served.map_err(|e| anyhow::anyhow!(e));
    }

    let mcp_settings: Arc<dyn SettingsRepository> = settings_repo.clone();
    let mcp_default = config.mcp.enabled.unwrap_or(config.auth.required);
    if !config.auth.required && folio_mcp::is_enabled(mcp_settings.as_ref(), mcp_default).await {
        tracing::warn!(
            "MCP endpoint /mcp is enabled without authentication (auth.required is false); \
             do not expose this server beyond localhost"
        );
    }

    // Create application state
    let health_checks: Vec<Arc<dyn HealthCheck>> =
        vec![Arc::new(DatabaseHealthCheck::new(database.clone()))];

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
        store,
        event_broadcaster,
    )
    .with_health_checks(health_checks);

    let auth = AuthState::new(auth_store, config.server.public_url.clone())
        .with_required(config.auth.required);

    // Create router
    let mcp = folio_mcp::router(
        library,
        mcp_settings,
        McpOptions {
            enabled_by_default: mcp_default,
            allowed_hosts: config.mcp.allowed_hosts.clone(),
        },
    );
    let mcp = if config.auth.required {
        mcp.layer(axum::middleware::from_fn(folio_mcp::bridge_auth_context))
            .layer(axum::middleware::from_fn_with_state(
                auth.validator("/mcp"),
                folio_auth::require_bearer,
            ))
    } else {
        mcp
    };
    let app = create_router(state, auth).merge(mcp);

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

    // Flush pending spans, metrics and logs; the exporters block while sending.
    let _ = tokio::task::spawn_blocking(move || telemetry.shutdown()).await;
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

/// Hide credentials in a database URL before logging it.
fn redact_url(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(scheme_end), Some(at)) if scheme_end < at => {
            format!("{}://***@{}", &url[..scheme_end], &url[at + 1..])
        }
        _ => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::redact_url;

    #[test]
    fn redacts_credentials() {
        assert_eq!(
            redact_url("postgres://folio:secret@db:5432/folio"),
            "postgres://***@db:5432/folio"
        );
        assert_eq!(
            redact_url("sqlite:folio.db?mode=rwc"),
            "sqlite:folio.db?mode=rwc"
        );
    }
}
