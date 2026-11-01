//! API routes

mod assets;
pub mod auth;
pub(crate) mod correspondents;
pub(crate) mod document_types;
pub(crate) mod documents;
pub(crate) mod events;
mod health;
mod manage;
mod pages;
mod pwa;
mod settings;
pub(crate) mod tags;
mod view;
mod workflow_pages;
pub(crate) mod workflows;

pub use events::turbo_streams_for;

use axum::{extract::DefaultBodyLimit, Router};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

pub use health::HealthChecks;

use crate::state::AppState;

/// Create the main application router
pub fn create_router(state: AppState, auth: auth::AuthState) -> Router {
    let health = health::router(state.health_checks.clone());
    let enforcement = axum::middleware::from_fn_with_state(auth.clone(), auth::enforce_auth);
    let forward = axum::middleware::from_fn_with_state(auth.clone(), crate::forward::forward_auth);
    let auth_routes = auth::router(auth);

    let router = Router::new()
        // API routes
        .nest("/api", api_router())
        // OpenAPI spec and docs viewer
        .merge(crate::openapi::router())
        // Page routes (HTMX)
        .merge(pages::router())
        // Manifest, service worker and share target
        .merge(pwa::router())
        // Management routes (correspondents, document types, tags)
        .merge(manage::router())
        // Settings routes
        .merge(settings::router())
        // Workflow pages
        .merge(workflow_pages::router())
        // SSE events
        .merge(events::router())
        // Login, setup and OAuth authorization server
        .merge(auth_routes)
        // Static files
        .merge(assets::router())
        // Middleware
        .layer(enforcement)
        .layer(forward)
        .layer(TraceLayer::new_for_http())
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

    router
}

/// API router for JSON endpoints
fn api_router() -> Router<AppState> {
    Router::new()
        .nest("/documents", documents::router())
        .nest("/tags", tags::router())
        .nest("/correspondents", correspondents::router())
        .nest("/document_types", document_types::router())
        .nest("/workflows", workflows::router())
}
