//! Installable-app routes: manifest, service worker and the share target

use axum::{
    extract::{Multipart, State},
    http::header,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};

use crate::error::ApiResult;
use crate::state::AppState;

use super::pages::store_upload;

const MANIFEST: &str = include_str!("../../../../static/manifest.webmanifest");
const SERVICE_WORKER: &str = include_str!("../../../../static/sw.js");

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/manifest.webmanifest", get(manifest))
        .route("/sw.js", get(service_worker))
        .route("/share", post(share_target))
}

async fn manifest() -> Response {
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        MANIFEST,
    )
        .into_response()
}

/// Served from the root so its scope covers the whole app
async fn service_worker() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
            (
                header::HeaderName::from_static("service-worker-allowed"),
                "/",
            ),
        ],
        SERVICE_WORKER,
    )
        .into_response()
}

/// Receives files shared to the installed app and files them into the inbox
async fn share_target(State(state): State<AppState>, multipart: Multipart) -> ApiResult<Response> {
    store_upload(&state, multipart).await?;
    Ok(Redirect::to("/").into_response())
}
