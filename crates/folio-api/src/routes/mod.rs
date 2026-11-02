//! API routes

mod assets;
pub mod auth;
pub(crate) mod browse;
pub(crate) mod correspondents;
pub(crate) mod document_entries;
pub(crate) mod document_types;
pub(crate) mod documents;
mod error_pages;
pub(crate) mod events;
mod file_response;
mod health;
mod integrations;
mod manage;
mod mfa;
mod oidc;
mod pages;
mod pending;
mod pwa;
mod security;
mod settings;
mod source_pages;
pub(crate) mod sources;
pub(crate) mod tags;
pub(crate) mod tokens;
pub(crate) mod trash;
pub(crate) mod vaults;
mod view;
mod workflow_pages;
pub(crate) mod workflows;

pub use events::turbo_streams_for;

use axum::{extract::DefaultBodyLimit, routing::get, Router};
use tower_http::cors::CorsLayer;

pub use health::HealthChecks;

use crate::state::AppState;

fn parse_classification_id(
    value: Option<String>,
    field: &str,
) -> crate::error::ApiResult<Option<uuid::Uuid>> {
    let value = value.ok_or_else(|| {
        crate::error::ApiError::bad_request(format!(
            "{field} is required; submit an empty value to clear it"
        ))
    })?;
    if value.is_empty() {
        Ok(None)
    } else {
        uuid::Uuid::parse_str(&value).map(Some).map_err(|_| {
            crate::error::ApiError::bad_request(format!("{field} must be a UUID or empty"))
        })
    }
}

/// Create the main application router
pub fn create_router(state: AppState, auth: auth::AuthState) -> Router {
    let health = health::router(state.health_checks.clone());
    let enforcement = axum::middleware::from_fn_with_state(auth.clone(), auth::enforce_auth);
    let forward = axum::middleware::from_fn_with_state(auth.clone(), crate::forward::forward_auth);
    let default_vault = axum::middleware::from_fn_with_state(
        crate::vault::VaultResolver {
            auth: auth.clone(),
            state: state.clone(),
        },
        crate::vault::resolve_default_vault,
    );
    let security = security::router(auth.clone());
    let settings = settings::router(auth.clone());
    let tokens = tokens::router(auth.clone());
    let auth_routes = auth::router(auth);

    let routes = Router::new()
        // API routes
        .nest("/api", api_router())
        // OpenAPI spec and docs viewer
        .merge(crate::openapi::router())
        // Page routes (HTMX)
        .merge(pages::router())
        .merge(trash::router())
        // Manifest, service worker and share target
        .merge(pwa::router())
        // Management routes (correspondents, document types, tags)
        .merge(manage::router())
        // Settings routes
        .merge(settings)
        .merge(integrations::router())
        // Account and security
        .merge(security)
        .merge(tokens)
        // Workflow pages
        .merge(workflow_pages::router())
        // Source pages
        .merge(source_pages::router())
        // SSE events
        .merge(events::router())
        // Login, setup and OAuth authorization server
        .merge(auth_routes)
        // Static files
        .merge(assets::router())
        // Unknown paths answer 404 through the layers, so browsers get an HTML page
        .fallback(|| async { axum::http::StatusCode::NOT_FOUND });

    let html_errors = axum::middleware::from_fn_with_state(state.clone(), error_pages::html_errors);

    let router = crate::telemetry::instrument(routes)
        // Middleware
        .layer(html_errors)
        .layer(default_vault)
        .layer(enforcement)
        .layer(forward)
        .layer(CorsLayer::permissive())
        // Allow larger file uploads (50MB)
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024))
        // State
        .with_state(state)
        // Probes stay unauthenticated: merged at the root, outside any future auth layer
        .merge(health);

    // Add livereload in debug mode
    #[cfg(debug_assertions)]
    let router = router.layer(tower_livereload::LiveReloadLayer::new());

    crate::vault::scoped_router(router)
}

/// API router for JSON endpoints
fn api_router() -> Router<AppState> {
    Router::new()
        .route("/vaults", axum::routing::get(vaults::list_vaults))
        .nest("/telemetry", crate::client_telemetry::router())
        .nest("/documents", documents::router())
        .route("/documents/browse", get(browse::browse_default))
        .route("/vaults/{vault_id}/documents", get(browse::browse_vault))
        .nest("/tags", tags::router())
        .nest("/correspondents", correspondents::router())
        .nest("/document_types", document_types::router())
        .nest("/workflows", workflows::router())
        .nest("/sources", sources::router())
}
