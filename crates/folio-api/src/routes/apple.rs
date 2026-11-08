//! Apple app site association, so self-built Folio apps can open this server's links

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use folio_core::models::AppleAppId;
use serde_json::{json, Value};

use crate::state::AppState;

/// Matches exactly one UUID path segment; Apple patterns treat `?` as one character
const UUID_MASK: &str = "????????-????-????-????-????????????";

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/.well-known/apple-app-site-association",
        get(app_site_association),
    )
}

async fn app_site_association(State(state): State<AppState>) -> Response {
    let app_ids = &state.apple_app_ids_default;
    if app_ids.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [(header::CACHE_CONTROL, "public, max-age=3600")],
        Json(association(app_ids, &path_prefix(&state.public_url))),
    )
        .into_response()
}

/// The path of `public_url` without a trailing slash; empty at the domain root
fn path_prefix(public_url: &str) -> String {
    url::Url::parse(public_url)
        .map(|url| url.path().trim_end_matches('/').to_string())
        .unwrap_or_default()
}

/// Claims only the pages Handoff hands over: a document, a saved view and a vault library
fn association(app_ids: &[AppleAppId], prefix: &str) -> Value {
    let u = UUID_MASK;
    let app_ids: Vec<&str> = app_ids.iter().map(AppleAppId::as_str).collect();
    json!({
        "applinks": {
            "details": [{
                "appIDs": app_ids,
                "components": [
                    { "/": format!("{prefix}/vaults/{u}/documents/{u}") },
                    { "/": format!("{prefix}/vaults/{u}/saved-views/{u}") },
                    { "/": format!("{prefix}/vaults/{u}/") },
                ]
            }]
        }
    })
}
