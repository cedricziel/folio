//! Settings page routes

use askama::Template;
use axum::{
    extract::State,
    http::header,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Form, Router,
};
use serde::Deserialize;
use tracing::info;

use folio_core::models::AiSettings;

use crate::error::ApiResult;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings", get(settings_page))
        .route("/settings/ai", post(save_ai_settings))
}

/// Settings page template
#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
    title: String,
    ai_settings: AiSettings,
}

/// Settings page
async fn settings_page(State(state): State<AppState>) -> ApiResult<Html<String>> {
    let settings = state.settings_repo.list_by_prefix("ai.").await?;
    let ai_settings = AiSettings::from_settings(&settings);

    let template = SettingsTemplate {
        title: "Settings - Folio".to_string(),
        ai_settings,
    };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering settings page: {}", e)
    })))
}

/// AI settings form data
#[derive(Debug, Deserialize)]
pub struct AiSettingsForm {
    provider: String,
    openai_api_key: String,
    anthropic_api_key: String,
    ocr_model: String,
    embedding_model: String,
}

/// Settings saved template (Turbo Stream response)
#[derive(Template)]
#[template(path = "partials/settings_saved.html")]
struct SettingsSavedTemplate {
    ai_settings: AiSettings,
}

/// Save AI settings
async fn save_ai_settings(
    State(state): State<AppState>,
    Form(form): Form<AiSettingsForm>,
) -> ApiResult<Response> {
    let ai_settings = AiSettings {
        provider: form.provider,
        openai_api_key: form.openai_api_key,
        anthropic_api_key: form.anthropic_api_key,
        ocr_model: form.ocr_model,
        embedding_model: form.embedding_model,
    };

    // Save all settings
    state
        .settings_repo
        .set_many(&ai_settings.to_settings())
        .await?;

    info!("AI settings saved: provider={}", ai_settings.provider);

    let template = SettingsSavedTemplate { ai_settings };

    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error rendering settings saved: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/vnd.turbo-stream.html")], html).into_response())
}
