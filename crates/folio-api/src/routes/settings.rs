//! Settings page routes

use crate::vault::filters;
use askama::Template;
use axum::{
    extract::{Query, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
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

use super::auth::AuthState;
use super::view::{Nav, Sidebar};

pub fn router(auth: AuthState) -> Router<AppState> {
    Router::new()
        .route("/settings", get(settings_page))
        .route("/settings/ai", post(save_ai_settings))
        .route("/settings/processing", post(save_processing_settings))
        .route("/settings/processing/test", post(test_processing_settings))
        .route("/settings/processing/reembed", post(reembed_all))
        .route("/settings/mcp", post(save_mcp_settings))
        .route("/settings/push", post(save_push_settings))
        .route("/settings/push/test", post(test_push_settings))
        .route_layer(middleware::from_fn_with_state(auth, require_admin))
}

async fn require_admin(State(auth): State<AuthState>, request: Request, next: Next) -> Response {
    if !auth.required {
        return next.run(request).await;
    }
    match auth.current_user(request.headers()).await {
        Some(user) if user.is_owner || user.is_admin => next.run(request).await,
        _ => StatusCode::FORBIDDEN.into_response(),
    }
}

struct CredentialPresence {
    openai_configured: bool,
    anthropic_configured: bool,
}

impl From<&AiSettings> for CredentialPresence {
    fn from(settings: &AiSettings) -> Self {
        Self {
            openai_configured: !settings.openai_api_key.is_empty(),
            anthropic_configured: !settings.anthropic_api_key.is_empty(),
        }
    }
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
    credentials: CredentialPresence,
    processing: PipelineSettings,
    ocr_providers: Vec<ProviderOption>,
    embedding_providers: Vec<ProviderOption>,
    notice: String,
    mcp_enabled: bool,
    push_status: Option<folio_infra::push::PushStatus>,
    push_devices: u64,
    push_notice: String,
}

/// Database key behind the MCP toggle; `folio-mcp` reads the same row
const MCP_ENABLED: &str = "mcp.enabled";

#[derive(Debug, Deserialize)]
struct SettingsQuery {
    reembedded: Option<usize>,
    saved: Option<String>,
    push: Option<String>,
}

/// Settings page
async fn settings_page(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Query(query): Query<SettingsQuery>,
) -> ApiResult<Html<String>> {
    let settings = state.settings_repo.list().await?;
    let mut processing = (*state.pipeline.settings().await).clone();
    processing.ai = AiSettings::default();
    let mcp_enabled = settings
        .iter()
        .find(|s| s.key == MCP_ENABLED)
        .map(|s| s.value.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(state.mcp_default);

    let push_notice = match query.push.as_deref() {
        Some("saved") => "Push settings saved.",
        Some("bad_key") => "Not saved: the key is not a PKCS#8 P-256 key (.p8).",
        Some("key_refused") => {
            "Not saved: set FOLIO_AUTH__SECRET_KEY before storing a key here, or give the key as FOLIO_PUSH__APNS__PRIVATE_KEY_FILE."
        }
        Some("bad_transport") => "Not saved: unknown transport.",
        Some("test_sent") => "Test notification sent to your registered devices.",
        Some("test_none") => "No test sent: none of your devices can receive one right now.",
        _ => "",
    }
    .to_string();
    let (push_status, push_devices) = match &state.push {
        Some(push) => (
            Some(push.settings.status().await),
            push.devices.count_devices().await?,
        ),
        None => (None, 0),
    };
    let notice = match (query.reembedded, query.saved) {
        (Some(n), _) => format!("Queued {n} documents for re-embedding."),
        (None, Some(section)) if section == "mcp" => "MCP setting saved.".to_string(),
        (None, Some(section)) if section == "push" => String::new(),
        (None, Some(_)) => "Processing settings saved.".to_string(),
        _ => String::new(),
    };

    let template = SettingsTemplate {
        sidebar: Sidebar::load(&state, vault, Nav::Settings).await?,
        title: "Settings - Folio".to_string(),
        credentials: CredentialPresence::from(&AiSettings::from_settings(&settings)),
        ocr_providers: options(state.pipeline.ocr_providers(), &processing.ocr.provider),
        embedding_providers: options(
            state.pipeline.embedding_providers(),
            &processing.embeddings.provider,
        ),
        processing,
        notice,
        mcp_enabled,
        push_status,
        push_devices,
        push_notice,
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
    clear_openai_api_key: Option<String>,
    clear_anthropic_api_key: Option<String>,
}

/// Settings saved template (Turbo Stream response)
#[derive(Template)]
#[template(path = "partials/settings_saved.html")]
struct SettingsSavedTemplate {
    credentials: CredentialPresence,
}

/// Save AI credentials
async fn save_ai_settings(
    State(state): State<AppState>,
    Form(form): Form<AiSettingsForm>,
) -> ApiResult<Response> {
    let mut updates = Vec::new();
    for (key, value, clear) in [
        (
            "ai.openai_api_key",
            form.openai_api_key,
            form.clear_openai_api_key,
        ),
        (
            "ai.anthropic_api_key",
            form.anthropic_api_key,
            form.clear_anthropic_api_key,
        ),
    ] {
        if clear.is_some() {
            updates.push((key.to_string(), String::new()));
        } else if !value.trim().is_empty() {
            updates.push((key.to_string(), value));
        }
    }
    state.settings_repo.set_many(&updates).await?;
    state.pipeline.invalidate().await;

    info!("AI credentials saved");

    let settings = state.settings_repo.list().await?;
    let html = SettingsSavedTemplate {
        credentials: CredentialPresence::from(&AiSettings::from_settings(&settings)),
    }
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
            .enqueue_untraced(JobType::generate_embedding(*document_id))
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

/// Push section form data
#[derive(Debug, Deserialize)]
pub struct PushForm {
    transport: String,
    #[serde(default)]
    team_id: String,
    #[serde(default)]
    key_id: String,
    #[serde(default)]
    topic: String,
    #[serde(default)]
    private_key: String,
    clear_private_key: Option<String>,
    quiet_window_seconds: Option<u32>,
    max_alerts_per_hour: Option<u32>,
}

/// Save push settings; they apply at once, without a restart
async fn save_push_settings(
    State(state): State<AppState>,
    Form(form): Form<PushForm>,
) -> ApiResult<Response> {
    use folio_infra::push;
    let Some(push_state) = &state.push else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let configured = &push_state.settings;
    if !matches!(form.transport.as_str(), "none" | "apns") {
        return Ok(Redirect::to("/settings?push=bad_transport").into_response());
    }
    let key = form.private_key.trim();
    let mut rows = vec![
        (push::TRANSPORT.to_string(), form.transport.clone()),
        (push::TEAM_ID.to_string(), form.team_id.trim().to_string()),
        (push::KEY_ID.to_string(), form.key_id.trim().to_string()),
        (push::TOPIC.to_string(), form.topic.trim().to_string()),
    ];
    if let Some(seconds) = form.quiet_window_seconds {
        rows.push((
            push::QUIET_WINDOW.to_string(),
            seconds.min(3600).to_string(),
        ));
    }
    if let Some(cap) = form.max_alerts_per_hour {
        rows.push((push::MAX_ALERTS.to_string(), cap.clamp(1, 3600).to_string()));
    }
    if !key.is_empty() {
        let (true, Some(secret_box)) = (configured.can_store_key(), configured.secret_box()) else {
            return Ok(Redirect::to("/settings?push=key_refused").into_response());
        };
        if push::apns::parse_key(key).is_err() {
            return Ok(Redirect::to("/settings?push=bad_key").into_response());
        }
        rows.push((
            push::PRIVATE_KEY.to_string(),
            push::seal_key(secret_box, key),
        ));
    } else if form.clear_private_key.is_some() {
        rows.push((push::PRIVATE_KEY.to_string(), String::new()));
    }
    state.settings_repo.set_many(&rows).await?;
    configured.invalidate().await;
    info!(transport = %form.transport, "Push settings saved");
    Ok(Redirect::to("/settings?push=saved").into_response())
}

/// Send a test notification to the signed-in admin's devices
async fn test_push_settings(
    State(state): State<AppState>,
    crate::vault::Uploader(user): crate::vault::Uploader,
) -> ApiResult<Response> {
    let (Some(push), Some(user)) = (&state.push, user) else {
        return Ok(Redirect::to("/settings?push=test_none").into_response());
    };
    let mut sent = false;
    for device in push.devices.list_for_user(&user).await? {
        sent |= push
            .sender
            .send_test(&user, &device.installation_id, chrono::Utc::now())
            .await
            .is_ok();
    }
    let outcome = if sent { "test_sent" } else { "test_none" };
    Ok(Redirect::to(&format!("/settings?push={outcome}")).into_response())
}
