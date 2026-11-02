mod common;

use axum::http::StatusCode;
use folio_api::{create_router, AuthState};
use folio_infra::{config::Config, PipelineContext};
use tower::ServiceExt;

#[tokio::test]
async fn settings_page_shows_effective_startup_pipeline_values() {
    let harness = common::harness().await;
    let mut state = harness.state;
    let mut config = Config::default();
    config.ocr.provider = "none".into();
    config.ocr.model = "configured-vision".into();
    config.processing.auto_ocr = false;
    config.ai.openai_api_key = Some("startup-only-key".into());
    state.pipeline = PipelineContext::from_config(state.settings_repo.clone(), &config);
    let app = create_router(
        state,
        AuthState::new(harness.auth_store, None).with_required(false),
    );
    let response = app.oneshot(common::get("/settings")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = common::text(response).await;
    assert!(html.contains("<option value=\"none\" selected>none</option>"));
    assert!(html.contains("value=\"configured-vision\""));
    assert!(html.contains("name=\"auto_ocr\""));
    assert!(!html.contains("name=\"auto_ocr\" checked"));
    assert!(!html.contains("startup-only-key"));
}
