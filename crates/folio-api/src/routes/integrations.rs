use askama::Template;
use axum::{
    extract::State,
    http::header,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};

use super::view::{Nav, Sidebar};
use crate::{
    error::ApiResult,
    state::AppState,
    vault::{filters, DefaultVault},
};

pub fn router() -> Router<AppState> {
    Router::new().route("/settings/integrations", get(page))
}

#[derive(Template)]
#[template(path = "integrations.html")]
struct IntegrationsTemplate {
    sidebar: Sidebar,
    mcp_enabled: bool,
    mcp_url: String,
}

async fn page(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
) -> ApiResult<Response> {
    let mcp_enabled = match state.settings_repo.get("mcp.enabled").await {
        Ok(Some(setting)) => setting.value.trim().eq_ignore_ascii_case("true"),
        _ => state.mcp_default,
    };
    let template = IntegrationsTemplate {
        sidebar: Sidebar::load(&state, vault, Nav::None).await?,
        mcp_enabled,
        mcp_url: format!("{}/mcp", state.public_url.trim_end_matches('/')),
    };
    let html = template
        .render()
        .map_err(|error| crate::error::ApiError::internal(error.to_string()))?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Html(html)).into_response())
}
