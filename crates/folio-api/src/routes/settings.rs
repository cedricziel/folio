//! Settings page routes

use askama::Template;
use axum::{
    extract::{Query, State},
    http::header,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use serde::Deserialize;
use tracing::{error, info};

use folio_core::models::{AiSettings, EmbeddingSettings, OcrSettings, PipelineSettings};
use folio_infra::JobType;

use crate::error::ApiResult;
use crate::state::AppState;
use crate::vault::DefaultVault;

use super::view::{Nav, Sidebar};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings", get(settings_page))
        .route("/settings/ai", post(save_ai_settings))
        .route("/settings/processing", post(save_processing_settings))
        .route("/settings/processing/test", post(test_processing_settings))
        .route("/settings/processing/reembed", post(reembed_all))
        .route("/settings/mcp", post(save_mcp_settings))
}

/// A provider choice in a `<select>`
struct ProviderOption {
    id: &'static str,
    selected: bool,
}

fn options(ids: Vec<&'static str>, current: &str) -> Vec<ProviderOption> {
    ids.into_iter()
        .map(|id| ProviderOption {
            id,
            selected: id == current,
        })
        .collect()
}

/// Settings page template
#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsTemplate {
    sidebar: Sidebar,
    title: String,
    ai_settings: AiSettings,
    processing: PipelineSettings,
    ocr_providers: Vec<ProviderOption>,
    embedding_providers: Vec<ProviderOption>,
    notice: String,
    mcp_enabled: bool,
}

/// Database key behind the MCP toggle; `folio-mcp` reads the same row
const MCP_ENABLED: &str = "mcp.enabled";

#[derive(Debug, Deserialize)]
struct SettingsQuery {
    reembedded: Option<usize>,
    saved: Option<String>,
}

/// Settings page
async fn settings_page(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Query(query): Query<SettingsQuery>,
) -> ApiResult<Html<String>> {
    let settings = state.settings_repo.list().await?;
    let processing = (*state.pipeline.settings().await).clone();
    let mcp_enabled = settings
        .iter()
        .find(|s| s.key == MCP_ENABLED)
        .map(|s| s.value.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(state.mcp_default);

    let notice = match (query.reembedded, query.saved) {
        (Some(n), _) => format!("Queued {n} documents for re-embedding."),
        (None, Some(section)) if section == "mcp" => "MCP setting saved.".to_string(),
        (None, Some(_)) => "Processing settings saved.".to_string(),
        _ => String::new(),
    };

    let template = SettingsTemplate {
        sidebar: Sidebar::load(&state, vault, Nav::Settings).await?,
        title: "Settings - Folio".to_string(),
        ai_settings: AiSettings::from_settings(&settings),
        ocr_providers: options(state.pipeline.ocr_providers(), &processing.ocr.provider),
        embedding_providers: options(
            state.pipeline.embedding_providers(),
            &processing.embeddings.provider,
        ),
        processing,
        notice,
        mcp_enabled,
    };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering settings page: {}", e)
    })))
}

/// AI credentials form data
#[derive(Debug, Deserialize)]
pub struct AiSettingsForm {
    openai_api_key: String,
    anthropic_api_key: String,
}

/// Settings saved template (Turbo Stream response)
#[derive(Template)]
#[template(path = "partials/settings_saved.html")]
struct SettingsSavedTemplate {
    ai_settings: AiSettings,
}

/// Save AI credentials
async fn save_ai_settings(
    State(state): State<AppState>,
    Form(form): Form<AiSettingsForm>,
) -> ApiResult<Response> {
    let ai_settings = AiSettings {
        openai_api_key: form.openai_api_key,
        anthropic_api_key: form.anthropic_api_key,
    };

    state
        .settings_repo
        .set_many(&ai_settings.to_settings())
        .await?;
    state.pipeline.invalidate().await;

    info!("AI credentials saved");

    let html = SettingsSavedTemplate { ai_settings }
        .render()
        .unwrap_or_else(|e| format!("Error rendering settings saved: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/vnd.turbo-stream.html")], html).into_response())
}

/// Processing section form data
#[derive(Debug, Deserialize)]
pub struct ProcessingForm {
    ocr_provider: String,
    #[serde(default)]
    ocr_model: String,
    #[serde(default)]
    ocr_languages: String,
    #[serde(default)]
    ocr_base_url: String,
    embeddings_provider: String,
    #[serde(default)]
    embeddings_model: String,
    #[serde(default)]
    embeddings_base_url: String,
    auto_ocr: Option<String>,
    auto_embed: Option<String>,
}

impl ProcessingForm {
    fn ocr(&self) -> OcrSettings {
        OcrSettings {
            provider: self.ocr_provider.trim().to_string(),
            model: self.ocr_model.trim().to_string(),
            languages: self.ocr_languages.trim().to_string(),
            base_url: self.ocr_base_url.trim().to_string(),
        }
    }

    fn embeddings(&self) -> EmbeddingSettings {
        EmbeddingSettings {
            provider: self.embeddings_provider.trim().to_string(),
            model: self.embeddings_model.trim().to_string(),
            base_url: self.embeddings_base_url.trim().to_string(),
        }
    }
}

/// Save OCR and embedding provider settings
async fn save_processing_settings(
    State(state): State<AppState>,
    Form(form): Form<ProcessingForm>,
) -> ApiResult<Response> {
    let ocr = form.ocr();
    let embeddings = form.embeddings();
    let flag = |on: &Option<String>| if on.is_some() { "true" } else { "false" }.to_string();

    state
        .settings_repo
        .set_many(&[
            ("ocr.provider".to_string(), ocr.provider),
            ("ocr.model".to_string(), ocr.model),
            ("ocr.languages".to_string(), ocr.languages),
            ("ocr.base_url".to_string(), ocr.base_url),
            ("embeddings.provider".to_string(), embeddings.provider),
            ("embeddings.model".to_string(), embeddings.model),
            ("embeddings.base_url".to_string(), embeddings.base_url),
            ("processing.auto_ocr".to_string(), flag(&form.auto_ocr)),
            ("processing.auto_embed".to_string(), flag(&form.auto_embed)),
        ])
        .await?;
    state.pipeline.invalidate().await;

    // Flag vectors of any other model as stale right away; search ignores them.
    match state.embedding_service.model_id().await {
        Ok(model) => {
            if let Err(e) = state.search_service.mark_stale_except(&model).await {
                error!("Failed to mark stale embeddings: {}", e);
            }
        }
        Err(e) => error!("Embedding provider is not usable: {}", e),
    }

    info!("Processing settings saved");
    Ok(Redirect::to("/settings?saved=1").into_response())
}

/// Frame shown after a connection test
#[derive(Template)]
#[template(path = "partials/processing_test.html")]
struct ProcessingTestTemplate {
    results: Vec<TestOutcome>,
}

struct TestOutcome {
    label: String,
    ok: bool,
    message: String,
}

impl TestOutcome {
    fn new(label: String, result: folio_core::error::Result<()>) -> Self {
        match result {
            Ok(()) => Self {
                label,
                ok: true,
                message: String::new(),
            },
            Err(e) => Self {
                label,
                ok: false,
                message: e.to_string(),
            },
        }
    }
}

/// Check the providers selected in the form, without saving them
async fn test_processing_settings(
    State(state): State<AppState>,
    Form(form): Form<ProcessingForm>,
) -> ApiResult<Html<String>> {
    let current = state.pipeline.settings().await;
    let mut candidate = (*current).clone();
    candidate.ocr = form.ocr();
    candidate.embeddings = form.embeddings();
    let candidate = state.pipeline.with_fallbacks(candidate);

    let ocr = match state.pipeline.build_ocr(&candidate) {
        Ok(service) => service.health().await,
        Err(e) => Err(e),
    };
    let embeddings = match state.pipeline.build_embeddings(&candidate) {
        Ok(service) => service.health().await,
        Err(e) => Err(e),
    };

    let results = vec![
        TestOutcome::new(format!("OCR ({})", candidate.ocr.provider), ocr),
        TestOutcome::new(
            format!("Embeddings ({})", candidate.embeddings.provider),
            embeddings,
        ),
    ];

    let html = ProcessingTestTemplate { results }
        .render()
        .unwrap_or_else(|e| format!("Error rendering test result: {}", e));
    Ok(Html(html))
}

/// Re-embed every document with the active embedding model
async fn reembed_all(
    DefaultVault(_vault): DefaultVault,
    State(state): State<AppState>,
) -> ApiResult<Response> {
    if let Ok(model) = state.embedding_service.model_id().await {
        state.search_service.mark_stale_except(&model).await?;
    }

    let ids = state.document_repo.reset_embedding_status().await?;
    for document_id in &ids {
        if let Err(e) = state
            .job_queue
            .enqueue(JobType::GenerateEmbedding {
                document_id: *document_id,
            })
            .await
        {
            error!("Failed to enqueue re-embedding for {}: {}", document_id, e);
        }
    }

    info!("Queued {} documents for re-embedding", ids.len());
    Ok(Redirect::to(&format!("/settings?reembedded={}", ids.len())).into_response())
}

/// MCP section form data
#[derive(Debug, Deserialize)]
pub struct McpForm {
    mcp_enabled: Option<String>,
}

/// Turn the MCP endpoint on or off; applies within seconds, without a restart
async fn save_mcp_settings(
    State(state): State<AppState>,
    Form(form): Form<McpForm>,
) -> ApiResult<Response> {
    let enabled = form.mcp_enabled.is_some();
    state
        .settings_repo
        .set(MCP_ENABLED, if enabled { "true" } else { "false" })
        .await?;

    info!(
        "MCP endpoint {}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(Redirect::to("/settings?saved=mcp").into_response())
}
