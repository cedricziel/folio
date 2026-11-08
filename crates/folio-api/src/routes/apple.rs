//! Apple app site association, so self-built Folio apps can open this server's links

use axum::{http::StatusCode, routing::get, Router};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/.well-known/apple-app-site-association",
        get(app_site_association),
    )
}

async fn app_site_association() -> StatusCode {
    StatusCode::NOT_FOUND
}
