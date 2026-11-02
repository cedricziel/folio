//! Streamable HTTP and stdio transports

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use folio_core::models::{UserId, VaultId, VaultRole};
use folio_core::ports::SettingsRepository;
use folio_core::services::{LibraryService, VaultService};
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

const SESSION_BINDING_IDLE: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_SESSION_BINDINGS: usize = 4096;

#[derive(Clone, PartialEq, Eq)]
struct SessionOwner {
    subject: Option<String>,
    client: Option<String>,
    personal_token: Option<String>,
    audience: Option<String>,
    scopes: Vec<String>,
    vault: VaultId,
    role: VaultRole,
}

#[derive(Clone)]
struct SessionGuard {
    vaults: Arc<VaultService>,
    bindings: Arc<Mutex<HashMap<String, (SessionOwner, Instant)>>>,
}

impl SessionGuard {
    async fn owner(
        &self,
        context: Option<folio_auth::AuthContext>,
        auth: Option<crate::McpAuth>,
    ) -> Result<SessionOwner, StatusCode> {
        let context = context.as_ref();
        let auth = auth.as_ref();
        let subject = context
            .map(|ctx| ctx.user_id.clone())
            .or_else(|| auth.and_then(|auth| auth.subject.clone()));
        let mut scopes: Vec<_> = context
            .map(|ctx| ctx.scopes.clone())
            .or_else(|| auth.map(|auth| auth.scopes.iter().cloned().collect()))
            .unwrap_or_default();
        scopes.sort();
        scopes.dedup();
        let user = subject.as_deref().map(UserId::from);
        let vault = self
            .vaults
            .default_vault(user.as_ref())
            .await
            .map_err(|_| StatusCode::FORBIDDEN)?;
        let role = self
            .vaults
            .list(user.as_ref())
            .await
            .map_err(|_| StatusCode::FORBIDDEN)?
            .into_iter()
            .find(|(item, _)| item.id == vault)
            .map(|(_, role)| role)
            .ok_or(StatusCode::FORBIDDEN)?;
        Ok(SessionOwner {
            subject,
            client: context.map(|ctx| ctx.client_id.clone()),
            personal_token: context.and_then(|ctx| ctx.personal_token_id.clone()),
            audience: context.map(|ctx| ctx.audience.clone()),
            scopes,
            vault,
            role,
        })
    }
}

async fn bind_session(State(guard): State<SessionGuard>, request: Request, next: Next) -> Response {
    let context = request
        .extensions()
        .get::<folio_auth::AuthContext>()
        .cloned();
    let auth = request.extensions().get::<crate::McpAuth>().cloned();
    let owner = match guard.owner(context, auth).await {
        Ok(owner) => owner,
        Err(status) => return status.into_response(),
    };
    let mut headers = request.headers().get_all("mcp-session-id").iter();
    let session = match headers.next() {
        Some(value) => match value.to_str() {
            Ok(value) => Some(value.to_string()),
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        },
        None => None,
    };
    if headers.next().is_some() {
        return StatusCode::NOT_FOUND.into_response();
    }
    {
        let mut bindings = guard.bindings.lock().unwrap();
        bindings.retain(|_, (_, at)| at.elapsed() < SESSION_BINDING_IDLE);
        if let Some(id) = &session {
            match bindings.get_mut(id) {
                Some((bound, at)) if *bound == owner => *at = Instant::now(),
                _ => return StatusCode::NOT_FOUND.into_response(),
            }
        }
    }
    let delete = request.method() == axum::http::Method::DELETE;
    let response = next.run(request).await;
    if response.status().is_success() {
        let mut bindings = guard.bindings.lock().unwrap();
        if let Some(id) = session.filter(|_| delete) {
            bindings.remove(&id);
        } else if let Some(id) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        {
            if !bindings.contains_key(id) && bindings.len() >= MAX_SESSION_BINDINGS {
                if let Some(oldest) = bindings
                    .iter()
                    .min_by_key(|(_, (_, at))| *at)
                    .map(|(id, _)| id.clone())
                {
                    bindings.remove(&oldest);
                }
            }
            bindings
                .entry(id.to_string())
                .or_insert_with(|| (owner, Instant::now()));
        }
    }
    response
}

/// Router serving the MCP endpoint at `/mcp`
///
/// Authentication is not built in: wrap the returned router in an auth layer
/// (`router(..).layer(auth)`) that inserts a [`crate::McpAuth`] into request
/// extensions.
pub fn router(
    library: Arc<LibraryService>,
    vaults: Arc<VaultService>,
    settings: Arc<dyn SettingsRepository>,
    options: McpOptions,
) -> Router {
    let mut config =
        StreamableHttpServerConfig::default().with_max_request_body_bytes(MAX_REQUEST_BODY_BYTES);
    config.allowed_hosts.extend(options.allowed_hosts);

    let guard = SessionGuard {
        vaults: vaults.clone(),
        bindings: Arc::default(),
    };
    let service = StreamableHttpService::new(
        move || Ok(FolioMcp::new(library.clone(), vaults.clone())),
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
        .layer(middleware::from_fn_with_state(guard, bind_session))
        .layer(middleware::from_fn_with_state(gate, enabled_gate))
}

/// Serve MCP over stdin/stdout until the client disconnects
pub async fn serve_stdio(
    library: Arc<LibraryService>,
    vaults: Arc<VaultService>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let running = FolioMcp::new(library, vaults)
        .serve(rmcp::transport::stdio())
        .await?;
    running.waiting().await?;
    Ok(())
}
