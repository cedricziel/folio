//! Streamable HTTP and stdio transports

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use folio_core::ports::SettingsRepository;
use folio_core::services::LibraryService;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::ServiceExt;

use crate::FolioMcp;

/// DB setting that overrides [`McpOptions::enabled_by_default`]
pub const ENABLED_SETTING: &str = "mcp.enabled";

/// Uploads arrive base64 encoded inside a JSON body
const MAX_REQUEST_BODY_BYTES: usize = 40 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct McpOptions {
    /// Whether `/mcp` is served when the `mcp.enabled` DB setting is absent
    pub enabled_by_default: bool,
    /// Accepted `Host` header values in addition to loopback, which rmcp always
    /// allows to protect local servers from DNS rebinding
    pub allowed_hosts: Vec<String>,
    /// How long a read of the `mcp.enabled` setting is reused, which is also
    /// how long a change in /settings takes to apply
    pub settings_ttl: Duration,
}

impl Default for McpOptions {
    fn default() -> Self {
        Self {
            enabled_by_default: true,
            allowed_hosts: Vec::new(),
            settings_ttl: Duration::from_secs(5),
        }
    }
}

#[derive(Clone)]
struct Gate {
    settings: Arc<dyn SettingsRepository>,
    enabled_by_default: bool,
    ttl: Duration,
    cached: Arc<Mutex<Option<(Instant, bool)>>>,
}

impl Gate {
    async fn enabled(&self) -> bool {
        let fresh = |(at, value): (Instant, bool)| (at.elapsed() < self.ttl).then_some(value);
        if let Some(value) = self.cached.lock().unwrap().and_then(fresh) {
            return value;
        }
        let value = is_enabled(self.settings.as_ref(), self.enabled_by_default).await;
        *self.cached.lock().unwrap() = Some((Instant::now(), value));
        value
    }
}

/// Whether `/mcp` is served: the DB setting wins over the configured default
pub async fn is_enabled(settings: &dyn SettingsRepository, default: bool) -> bool {
    match settings.get(ENABLED_SETTING).await {
        Ok(Some(setting)) => setting.value.trim().eq_ignore_ascii_case("true"),
        _ => default,
    }
}

async fn enabled_gate(State(gate): State<Gate>, req: Request, next: Next) -> Response {
    if gate.enabled().await {
        next.run(req).await
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

/// Router serving the MCP endpoint at `/mcp`
///
/// Authentication is not built in: wrap the returned router in an auth layer
/// (`router(..).layer(auth)`) that inserts a [`crate::McpAuth`] into request
/// extensions.
pub fn router(
    library: Arc<LibraryService>,
    settings: Arc<dyn SettingsRepository>,
    options: McpOptions,
) -> Router {
    let mut config =
        StreamableHttpServerConfig::default().with_max_request_body_bytes(MAX_REQUEST_BODY_BYTES);
    config.allowed_hosts.extend(options.allowed_hosts);

    let service = StreamableHttpService::new(
        move || Ok(FolioMcp::new(library.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );

    let gate = Gate {
        settings,
        enabled_by_default: options.enabled_by_default,
        ttl: options.settings_ttl,
        cached: Arc::default(),
    };
    Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(gate, enabled_gate))
}

/// Serve MCP over stdin/stdout until the client disconnects
pub async fn serve_stdio(
    library: Arc<LibraryService>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let running = FolioMcp::new(library)
        .serve(rmcp::transport::stdio())
        .await?;
    running.waiting().await?;
    Ok(())
}
