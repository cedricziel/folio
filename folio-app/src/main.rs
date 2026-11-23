//! Folio Application - Main entry point

mod healthcheck;
mod shutdown;

use std::future::IntoFuture;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::info;

use folio_api::{create_router, AppState, AuthState, IngestionState};
use folio_core::ports::{HealthCheck, RepositoryDocuments, SettingsRepository};
use folio_core::services::notifications::{NotificationPlanner, NotificationSender, SenderConfig};
use folio_core::services::{
    backfill_file_sizes, backfill_original_identities, BroadcastConsumer, DispatcherConfig,
    Importer, IngestPipeline, IngestScheduler, IngestWorkflowAccess, LibraryService,
    OutboxDispatcher, VaultService, WorkflowEngine,
};
use folio_core::EventBroadcaster;
use folio_infra::archival::{ArchiveConverter, ToolPaths};
use folio_infra::push::ConfiguredPush;
use folio_infra::queue::ArchiveService;
use folio_infra::{
    build_store, telemetry, Config, Database, DatabaseHealthCheck, DynamicEmbeddingService,
    DynamicOcrService, PipelineContext, SourceFactory, StorageFileMetadata, ThumbnailService,
    WebhookClient, Worker, WorkerConfig, WorkflowRunner,
};
use folio_mcp::McpOptions;

/// What the binary was asked to do
enum Mode {
    Server,
    McpStdio,
}

/// Database settings key of the generated secret key.
const SECRET_KEY_SETTING: &str = "auth.secret_key";

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
    let entry_repo = repos.entries;
    let workflow_document_repo = repos.workflow_documents;
    let outbox = repos.outbox;
    let tag_repo = repos.tags;
    let correspondent_repo = repos.correspondents;
    let document_type_repo = repos.document_types;
    let settings_repo = repos.settings;
    let workflow_repo = repos.workflows;
    let source_repo = repos.sources;
    let ledger = repos.ledger;
    let auth_store = repos.auth;
    let push_devices = repos.push.clone();
    let push_grants = repos.grants.clone();
    let push_access = repos.access.clone();
    let push_wake = Arc::new(tokio::sync::Notify::new());
    let vault_service = Arc::new(
        VaultService::new(repos.vaults.clone(), repos.members.clone()).with_audit(repos.audit),
    );

    let stored_documents = document_repo.list_all().await.map(|d| d.len()).unwrap_or(0);
    telemetry::metrics().documents_changed(stored_documents as i64);

    // Create AI services (providers follow the database settings, config.toml keys as fallback)
    let pipeline = PipelineContext::from_config(settings_repo.clone(), &config);
    let ocr_service = Arc::new(DynamicOcrService::new(pipeline.clone()));
    let embedding_service = Arc::new(DynamicEmbeddingService::new(pipeline.clone()));
    let search_service = repos.search;

    // Create job queue
    let job_queue = repos.jobs;

    // Live SSE connections are fed by the outbox dispatcher; the buffer only
    // bridges slow connections, which replay what they missed from the outbox.
    let event_broadcaster = EventBroadcaster::new(100);

    let configured_sources = config.ingest.definitions();
    let ingest_workflow_access = Arc::new(IngestWorkflowAccess::new(
        source_repo.clone(),
        repos.vaults.clone(),
        repos.members.clone(),
        &configured_sources,
        config.auth.required,
    ));
    let webhook_client = WebhookClient::new();
    let public_url = config.server.base_url();
    // Workflow edits are announced to live clients but never re-trigger workflows.
    let workflow_runner = Arc::new(
        WorkflowRunner::new(
            workflow_repo.clone(),
            workflow_document_repo,
            webhook_client.clone(),
        )
        .with_ingest_access(ingest_workflow_access.clone()),
    );
    let workflow_engine = Arc::new(
        WorkflowEngine::new(
            workflow_repo.clone(),
            document_repo.clone(),
            correspondent_repo.clone(),
            document_type_repo.clone(),
            Arc::new(StorageFileMetadata::new(store.clone())),
            public_url.clone(),
        )
        .with_ingest_access(ingest_workflow_access),
    );

    let source_factory = Arc::new(SourceFactory::new(ledger.clone()));
    let ingest_pipeline = Arc::new(
        IngestPipeline::new(
            document_repo.clone(),
            store.clone(),
            tag_repo.clone(),
            workflow_repo.clone(),
            ledger.clone(),
            Arc::new(Importer::new(
                document_repo.clone(),
                store.clone(),
                Arc::new(job_queue.clone()),
            )),
        )
        .with_outcomes(repos.ingest_outcomes.clone())
        .with_configured_sources(configured_sources.iter().map(|source| source.id)),
    );
    let ingest_scheduler = IngestScheduler::new(
        source_repo.clone(),
        configured_sources.clone(),
        source_factory.clone(),
        ingest_pipeline,
        repos.vaults,
    )
    .with_outcomes(repos.ingest_outcomes.clone());

    info!("Services initialized");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut background = JoinSet::new();

    let archive_service = Arc::new(ArchiveService::new(
        database.clone(),
        repos.archival.clone(),
        store.clone(),
        ArchiveConverter::with_tools(
            config.archival.languages.clone(),
            ToolPaths {
                verapdf: config.archival.verapdf.clone(),
                ..ToolPaths::default()
            },
        ),
        config.archival.automatic,
    ));

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
        outbox.clone(),
    )
    .with_workflow_runner(workflow_runner)
    .with_archive_service(archive_service.clone());

    background.spawn(ingest_scheduler.run(shutdown_tx.subscribe()));

    let (sized_documents, sized_store) = (document_repo.clone(), store.clone());
    background.spawn(async move {
        match backfill_file_sizes(sized_documents.as_ref(), sized_store.as_ref(), 200).await {
            Ok(0) => {}
            Ok(recorded) => info!("Recorded the file size of {recorded} documents"),
            Err(e) => tracing::warn!("File size backfill stopped: {e}"),
        }
        match backfill_original_identities(sized_documents.as_ref(), sized_store.as_ref(), 200)
            .await
        {
            Ok(0) => {}
            Ok(verified) => info!("Verified {verified} original file identities"),
            Err(e) => tracing::warn!("Original verification stopped: {e}"),
        }
    });

    background.spawn(async move {
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

    let secret_key = match config.auth.secret_key.as_deref().filter(|k| !k.is_empty()) {
        Some(key) => {
            if let Some(stored) = settings_repo.get(SECRET_KEY_SETTING).await? {
                if stored.value != key {
                    tracing::error!(
                        "auth.secret_key differs from the database fallback key; existing TOTP enrollments and recovery codes created with the stored key will stop working. Restore that key or reset those enrollments"
                    );
                }
            }
            key.to_string()
        }
        None => {
            tracing::warn!(
                "auth.secret_key is not set; using a key kept in the database. Set FOLIO_AUTH__SECRET_KEY (or FOLIO_AUTH__SECRET_KEY_FILE) so a copy of the database alone cannot decrypt two-factor secrets"
            );
            match settings_repo.get(SECRET_KEY_SETTING).await? {
                Some(setting) => setting.value,
                None => {
                    let key = folio_auth::secret::SecretBox::generate_key();
                    settings_repo.set(SECRET_KEY_SETTING, &key).await?;
                    key
                }
            }
        }
    };
    // Push notifications: the planner queues, the sender delivers.
    let external_secret = config
        .auth
        .secret_key
        .as_deref()
        .is_some_and(|k| !k.is_empty());
    let push_settings = Arc::new(ConfiguredPush::new(
        settings_repo.clone(),
        config.push.clone(),
        folio_auth::secret::SecretBox::new(&secret_key)
            .ok()
            .map(Arc::new),
        external_secret,
    ));
    let push_sender = Arc::new(NotificationSender::new(
        push_devices.clone(),
        Arc::new(RepositoryDocuments(document_repo.clone())),
        push_access.clone(),
        push_grants,
        push_settings.clone(),
        push_settings.clone(),
        SenderConfig::default(),
    ));
    // Delivers committed events to workflows and live clients, including those
    // left over from before a restart. Stops when the shutdown signal flips.
    let dispatcher = Arc::new(OutboxDispatcher::new(
        outbox.clone(),
        vec![
            Arc::new(BroadcastConsumer(event_broadcaster.clone())),
            workflow_engine,
            Arc::new(NotificationPlanner::new(
                push_devices.clone(),
                Arc::new(RepositoryDocuments(document_repo.clone())),
                push_access.clone(),
                push_settings.clone(),
                push_wake.clone(),
            )),
        ],
        DispatcherConfig::default(),
    ));
    background.spawn(dispatcher.run(shutdown_tx.subscribe()));
    background.spawn(
        push_sender
            .clone()
            .run(push_wake.clone(), shutdown_tx.subscribe()),
    );

    if matches!(mode, Mode::McpStdio) {
        let served = folio_mcp::serve_stdio(library, vault_service.clone()).await;
        shutdown_tx.send_replace(true);
        shutdown::wait_for_background(background).await;
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

    let mut state = AppState::new(
        repos.archival,
        repos.metadata_fields,
        repos.metadata_options,
        repos.metadata_values,
        repos.saved_views,
        document_repo.clone(),
        entry_repo,
        repos.upload_receipts,
        tag_repo,
        correspondent_repo,
        document_type_repo,
        settings_repo.clone(),
        ocr_service,
        embedding_service,
        search_service,
        pipeline,
        thumbnail_service,
        job_queue,
        store,
        event_broadcaster.clone(),
        outbox.clone(),
        workflow_repo,
        webhook_client,
        public_url,
        IngestionState::new(
            source_repo,
            ledger,
            repos.ingest_outcomes,
            configured_sources,
        ),
        vault_service.clone(),
        repos.access,
    )
    .with_health_checks(health_checks)
    .with_mcp_default(mcp_default)
    .with_apple_app_ids_default(config.apple.parsed().map_err(anyhow::Error::msg)?);
    state.archive = Some(archive_service);

    state.client_telemetry =
        folio_api::client_telemetry::ClientTelemetry::from_lookup(|key| std::env::var(key).ok())
            .map_err(|error| anyhow::anyhow!(error))?
            .map(Arc::new);

    let mut auth = AuthState::new(auth_store.clone(), Some(config.server.base_url()))
        .with_required(config.auth.required);
    if let Some(token) = config.auth.setup_token.as_deref().filter(|t| !t.is_empty()) {
        auth = auth.with_setup_token(token);
    }
    auth = auth
        .with_secret_key(&secret_key)
        .map_err(|e| anyhow::anyhow!("invalid auth.secret_key: {e}"))?;

    state.push = Some(folio_api::routes::devices::PushState {
        devices: push_devices,
        sender: push_sender,
        settings: push_settings,
    });
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
    let user_count = auth_store.count_users().await.unwrap_or(1);
    if setup_link_usable(config.auth.required, user_count, auth.local_login_allowed()) {
        announce_setup(
            &config.server.base_url(),
            auth.setup_token(),
            &mut std::io::stderr(),
        );
    }

    // Create router
    let mcp = folio_mcp::router(
        library,
        vault_service,
        mcp_settings,
        McpOptions {
            enabled_by_default: mcp_default,
            allowed_hosts: config.mcp.allowed_hosts.clone(),
            ..McpOptions::default()
        },
    );
    let mcp = mcp
        .layer(axum::middleware::from_fn_with_state(
            auth_store.clone(),
            folio_mcp::bridge_auth_context,
        ))
        .layer(axum::middleware::from_fn_with_state(
            auth.validator("/mcp").with_required(config.auth.required),
            folio_auth::require_bearer,
        ));
    let upload_tasks = state.upload_tasks.clone();
    background.spawn(sweep_upload_receipts(
        state.keyed_uploads.clone(),
        shutdown_tx.subscribe(),
    ));
    let cancellation = CancellationToken::new();
    let app =
        create_router(state, auth.clone())
            .merge(mcp)
            .layer(axum::middleware::from_fn_with_state(
                cancellation.clone(),
                shutdown::close_event_streams,
            ));

    // Start server
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .context(format!("Failed to bind to {}", addr))?;

    info!("🚀 Folio is running at http://{}", addr);

    let maintenance = auth.spawn_maintenance();
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(cancellation.clone().cancelled_owned())
    .into_future();
    tokio::pin!(server);

    let result = tokio::select! {
        result = &mut server => Some(result),
        () = shutdown_signal() => None,
    };
    cancellation.cancel();
    shutdown_tx.send_replace(true);
    let result = match result {
        Some(result) => Some(result),
        None => shutdown::drain_http(server).await,
    };
    if let Some(Err(e)) = result {
        tracing::error!("Server error: {}", e);
    }

    maintenance.abort();
    let _ = maintenance.await;
    // Let keyed uploads record their receipts so retries can replay them after a restart.
    upload_tasks.close();
    tokio::join!(shutdown::wait_for_background(background), async {
        if tokio::time::timeout(Duration::from_secs(30), upload_tasks.wait())
            .await
            .is_err()
        {
            tracing::warn!(
                "Stopping with uploads still running; their keys can be retried once they go stale"
            );
        }
    },);

    info!("Folio shutdown complete");

    // Flush pending spans, metrics and logs; the exporters block while sending.
    let _ = tokio::task::spawn_blocking(move || telemetry.shutdown()).await;
    Ok(())
}

/// Hourly, forget expired upload receipts and remove the unfinished documents they reserved
async fn sweep_upload_receipts(
    uploads: Arc<folio_core::services::IdempotentUploads>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
    loop {
        tokio::select! {
            _ = interval.tick() => match uploads.sweep(chrono::Utc::now()).await {
                Ok(0) => {}
                Ok(expired) => info!("Removed {expired} upload reservations that never completed"),
                Err(e) => tracing::warn!("Upload receipt sweep failed: {e}"),
            },
            _ = shutdown.changed() => return,
        }
    }
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

/// The setup link only works while auth is on, no user exists and the password form is allowed.
fn setup_link_usable(auth_required: bool, user_count: i64, local_login_allowed: bool) -> bool {
    auth_required && user_count == 0 && local_login_allowed
}

/// Prints the owner setup link to `console`, outside `tracing`, so the token
/// never reaches OTLP export, log files or span attributes. Stderr is used
/// because stdout carries the protocol in stdio mode.
fn announce_setup(base_url: &str, token: &str, console: &mut impl std::io::Write) {
    tracing::warn!(
        "No owner account exists yet; the setup link is printed once on standard error at startup"
    );
    let _ = writeln!(
        console,
        "\n\n  No owner account exists yet. Open this link to create it:\n\n    {}/setup?token={}\n\n  The link works until the first account is created.\n",
        base_url.trim_end_matches('/'),
        token
    );
}

#[cfg(test)]
mod tests {
    use super::{announce_setup, redact_url, setup_link_usable};
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    #[test]
    fn no_setup_link_when_local_login_is_off() {
        assert!(setup_link_usable(true, 0, true));
        assert!(!setup_link_usable(true, 0, false));
        assert!(!setup_link_usable(true, 1, true));
        assert!(!setup_link_usable(false, 0, true));
    }

    #[test]
    fn setup_token_stays_out_of_tracing() {
        let events = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(events.clone())
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let mut console = Vec::new();
        tracing::subscriber::with_default(subscriber, || {
            let _span = tracing::info_span!("startup").entered();
            announce_setup("https://folio.example.com/", "s3cr3t-token", &mut console);
        });

        let traced = String::from_utf8(events.0.lock().unwrap().clone()).unwrap();
        assert!(
            !traced.contains("s3cr3t-token"),
            "token reached tracing: {traced}"
        );
        assert!(
            !traced.contains("/setup"),
            "setup URL reached tracing: {traced}"
        );
        assert!(!traced.is_empty(), "setup should still be announced");

        let console = String::from_utf8(console).unwrap();
        assert!(console.contains("https://folio.example.com/setup?token=s3cr3t-token"));
        assert!(console.contains("until the first account is created"));
    }

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
