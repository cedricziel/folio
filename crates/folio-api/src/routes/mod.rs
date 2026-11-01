//! API routes

mod correspondents;
mod document_types;
mod documents;
mod events;
mod manage;
mod pages;
mod settings;
mod tags;
mod view;

use axum::{extract::DefaultBodyLimit, Router};
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};

use crate::state::AppState;

/// Create the main application router
pub fn create_router(state: AppState) -> Router {
    let router = Router::new()
        // API routes
        .nest("/api", api_router())
        // Page routes (HTMX)
        .merge(pages::router())
        // Management routes (correspondents, document types, tags)
        .merge(manage::router())
        // Settings routes
        .merge(settings::router())
        // SSE events
        .merge(events::router())
        // Static files
        .nest_service("/static", ServeDir::new("static"))
        // Middleware
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        // Allow larger file uploads (50MB)
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024))
        // State
        .with_state(state);

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
}
