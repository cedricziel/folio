//! Folio Application - Main entry point

mod healthcheck;

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::info;

use folio_api::{create_router, AppState, AuthState, IngestionState};
use folio_core::ports::{HealthCheck, SettingsRepository};
use folio_core::services::{
    EventingDocumentRepository, Importer, IngestPipeline, IngestScheduler, LibraryService,
    WorkflowEngine,
};
use folio_core::EventBroadcaster;
use folio_infra::{
    build_store, cleanup_legacy_layout, migrate_legacy_layout, telemetry, Config, Database,
    DatabaseHealthCheck, DynamicEmbeddingService, DynamicOcrService, PipelineContext,
    SourceFactory, StorageFileMetadata, ThumbnailService, WebhookClient, Worker, WorkerConfig,
    WorkflowRunner,
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

    if !matches!(mode, Mode::McpStdio) {
        config.validate().map_err(anyhow::Error::msg)?;
    }

    info!("Configuration loaded");
    info!("  Server: {}:{}", config.server.host, config.server.port);
    info!("  Database: {}", redact_url(&config.database.url));
    info!("  Storage: {:?}", config.storage.backend);

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
    let workflow_repo = repos.workflows;
    let source_repo = repos.sources;
    let ledger = repos.ledger;
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

    // Create AI services (providers follow the database settings, config.toml keys as fallback)
    let pipeline = PipelineContext::new(settings_repo.clone(), config.ai.clone());
    let ocr_service = Arc::new(DynamicOcrService::new(pipeline.clone()));
    let embedding_service = Arc::new(DynamicEmbeddingService::new(pipeline.clone()));
    let search_service = repos.search;

    // Create job queue
    let job_queue = repos.jobs;

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
        Arc::new(StorageFileMetadata::new(store.clone())),
        public_url.clone(),
    ));

    let source_factory = Arc::new(SourceFactory::new(ledger.clone()));
    let configured_sources = config.ingest.definitions();
    let ingest_pipeline = Arc::new(IngestPipeline::new(
        document_repo.clone(),
        tag_repo.clone(),
        workflow_repo.clone(),
        ledger.clone(),
        Arc::new(Importer::new(
            api_document_repo.clone(),
            store.clone(),
            Arc::new(job_queue.clone()),
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
        pipeline.clone(),
        thumbnail_service.clone(),
        store.clone(),
        WorkerConfig::default(),
        event_broadcaster.clone(),
    )
    .with_workflow_runner(workflow_runner);

    // Stops on its own when the shutdown signal flips
    tokio::spawn(workflow_engine.run(event_broadcaster.subscribe(), shutdown_tx.subscribe()));

    // Stops on its own when the shutdown signal flips
    tokio::spawn(ingest_scheduler.run(shutdown_tx.subscribe()));

    let worker_handle = tokio::spawn(async move {
        worker.run(shutdown_rx).await;
    });

    info!("Background job worker started");

    // MCP edits are user actions, so they go through the repository that announces them
    let library = Arc::new(LibraryService::new(
        api_document_repo.clone(),
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
        api_document_repo,
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
        workflow_repo,
        webhook_client,
        public_url,
        IngestionState::new(source_repo, ledger, configured_sources),
    )
    .with_health_checks(health_checks);

    let mut auth = AuthState::new(auth_store.clone(), Some(config.server.base_url()))
        .with_required(config.auth.required);
    if let Some(token) = config.auth.setup_token.as_deref().filter(|t| !t.is_empty()) {
        auth = auth.with_setup_token(token);
    }
    let forward = &config.auth.forward;
    if forward.enabled {
        let settings = folio_auth::ForwardAuth::new(folio_auth::ForwardConfig {
            user_header: forward.user_header.clone(),
            email_header: forward.email_header.clone(),
            name_header: forward.name_header.clone(),
            groups_header: forward.groups_header.clone(),
            role_header: forward.role_header.clone(),
            user_id_header: forward.user_id_header.clone(),
            trusted_proxies: forward.trusted_proxies.clone(),
            admin_group: forward.admin_group.clone(),
            admin_role: forward.admin_role.clone(),
            shared_secret: forward.shared_secret.clone(),
            secret_header: forward.secret_header.clone(),
            group_separator: forward.group_separator.clone(),
            session_ttl_secs: i64::try_from(forward.session_ttl_secs).unwrap_or(i64::MAX / 4),
        })
        .map_err(|e| anyhow::anyhow!("invalid auth.forward configuration: {e}"))?;
        if forward.admin_group.is_some() && forward.groups_header.is_empty() {
            tracing::warn!(
                "auth.forward.admin_group is ignored because auth.forward.groups_header is not set"
            );
        }
        if forward.admin_role.is_some() && forward.role_header.is_empty() {
            tracing::warn!(
                "auth.forward.admin_role is ignored because auth.forward.role_header is not set"
            );
        }
        if settings.trusts_everyone() {
            tracing::warn!(
                "auth.forward.trusted_proxies covers every address; any client can then impersonate any user. Trust only your proxy's address"
            );
        }
        if !settings.has_trusted_proxies() {
            tracing::warn!(
                "auth.forward is enabled without trusted_proxies; every request will be treated as unauthenticated"
            );
        }
        auth = auth.with_forward(settings);
    }

    let oidc = &config.auth.oidc;
    if oidc.enabled {
        let required = |value: &Option<String>, name: &str| {
            value
                .clone()
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("auth.oidc.enabled needs auth.oidc.{name}"))
        };
        let provider = folio_auth::oidc::OidcProvider::new(folio_auth::oidc::OidcSettings {
            issuer: required(&oidc.issuer, "issuer")?,
            client_id: required(&oidc.client_id, "client_id")?,
            client_secret: oidc.client_secret.clone().filter(|s| !s.is_empty()),
            scopes: oidc.scopes.split_whitespace().map(String::from).collect(),
            display_name: oidc.display_name.clone(),
            groups_claim: oidc.groups_claim.clone(),
            admin_group: oidc.admin_group.clone().filter(|g| !g.is_empty()),
            allow_local_login: oidc.allow_local_login,
            auto_create_users: oidc.auto_create_users,
        })
        .map_err(|e| anyhow::anyhow!("invalid auth.oidc configuration: {e}"))?;
        if let Err(e) = provider.warm_up().await {
            tracing::warn!("OpenID Connect discovery failed, will retry on first login: {e}");
        }
        auth = auth.with_oidc(provider);
    }
    if config.auth.required && auth_store.count_users().await.unwrap_or(1) == 0 {
        let base = config.server.base_url();
        tracing::warn!(
            "\n\n  No owner account exists yet. Open this link to create it:\n\n    {}/setup?token={}\n\n  The link works until the first account is created.\n",
            base.trim_end_matches('/'),
            auth.setup_token()
        );
    }

    // Create router
    let mcp = folio_mcp::router(
        library,
        mcp_settings,
        McpOptions {
            enabled_by_default: mcp_default,
            allowed_hosts: config.mcp.allowed_hosts.clone(),
        },
            ..McpOptions::default()
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
    let app = create_router(state, auth.clone()).merge(mcp);

    // Start server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context(format!("Failed to bind to {}", addr))?;

    info!("🚀 Folio is running at http://{}", addr);

    // Run server with graceful shutdown
    let maintenance = auth.spawn_maintenance();
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal());

    if let Err(e) = server.await {
        tracing::error!("Server error: {}", e);
    }

    maintenance.abort();
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
