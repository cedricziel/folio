mod common;

use axum::{body::Body, http::Request};
use folio_auth::{crypto::hash_token, model::NO_PASSWORD, Session, User};
use folio_core::models::{AiSettings, CreateDocument, ProcessingStage, ProcessingStatus, UserId};

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

async fn session(h: &common::Harness, admin: bool, owner: bool) -> String {
    let user = User {
        id: "settings-user".into(),
        username: "settings-user".into(),
        password_hash: NO_PASSWORD.into(),
        created_at: 1,
        email: None,
        display_name: None,
        is_admin: admin,
        is_owner: owner,
        auth_source: "oidc:https://identity.test".into(),
        external_id: Some("settings-user".into()),
    };
    if owner {
        assert!(
            h.auth_store
                .create_first_user(user)
                .await
                .unwrap()
                .unwrap()
                .is_owner
        );
    } else {
        h.auth_store.create_user(user).await.unwrap();
    }
    h.state
        .vaults
        .provision_user(&UserId::from("settings-user"))
        .await
        .unwrap();
    h.auth_store
        .create_session(Session {
            id_hash: hash_token("settings-test-session"),
            user_id: "settings-user".into(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    "folio_session=settings-test-session".into()
}

fn settings_request(method: &str, path: &str, cookie: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", cookie)
        .header("host", "localhost")
        .header("origin", "http://localhost")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn seed_keys(h: &common::Harness) {
    h.state
        .settings_repo
        .set_many(
            &AiSettings {
                openai_api_key: "stored-openai-secret".into(),
                anthropic_api_key: "stored-anthropic-secret".into(),
            }
            .to_settings(),
        )
        .await
        .unwrap();
}

async fn stored_keys(h: &common::Harness) -> AiSettings {
    AiSettings::from_settings(&h.state.settings_repo.list().await.unwrap())
}

async fn stored_settings(h: &common::Harness) -> Vec<(String, String)> {
    h.state
        .settings_repo
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|setting| (setting.key, setting.value))
        .collect()
}

async fn ordinary_user_is_denied(method: &str, path: &str, body: &str) {
    let h = common::harness_requiring_login().await;
    let cookie = session(&h, false, false).await;
    seed_keys(&h).await;
    let before = stored_settings(&h).await;

    let response = h
        .app
        .clone()
        .oneshot(settings_request(method, path, &cookie, body))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
    let html = common::text(response).await;
    assert!(!html.contains("stored-openai-secret"));
    assert!(!html.contains("stored-anthropic-secret"));
    assert_eq!(stored_settings(&h).await, before);
}

#[tokio::test]
async fn ordinary_user_cannot_read_global_settings() {
    ordinary_user_is_denied("GET", "/settings", "").await;
}

#[tokio::test]
async fn ordinary_user_cannot_replace_provider_keys() {
    ordinary_user_is_denied(
        "POST",
        "/settings/ai",
        "openai_api_key=changed&anthropic_api_key=changed",
    )
    .await;
}

#[tokio::test]
async fn ordinary_user_cannot_change_processing_settings() {
    ordinary_user_is_denied(
        "POST",
        "/settings/processing",
        "ocr_provider=none&embeddings_provider=none",
    )
    .await;
}

#[tokio::test]
async fn ordinary_user_cannot_test_providers() {
    ordinary_user_is_denied(
        "POST",
        "/settings/processing/test",
        "ocr_provider=none&embeddings_provider=none",
    )
    .await;
}

#[tokio::test]
async fn ordinary_user_cannot_toggle_mcp() {
    ordinary_user_is_denied("POST", "/settings/mcp", "mcp_enabled=on").await;
}

#[tokio::test]
async fn ordinary_user_cannot_reset_embeddings_in_another_vault() {
    let h = common::harness_requiring_login().await;
    let cookie = session(&h, false, false).await;
    let doc = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Another vault's document".into(),
                content: Some("Indexed contents".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.state
        .document_repo
        .set_stage_status(
            h.vault,
            doc.id,
            ProcessingStage::Embed,
            ProcessingStatus::Done,
        )
        .await
        .unwrap();

    let response = h
        .app
        .clone()
        .oneshot(settings_request(
            "POST",
            "/settings/processing/reembed",
            &cookie,
            "",
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let retained = h
        .state
        .document_repo
        .find_by_id(h.vault, doc.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.embed_status, ProcessingStatus::Done);
}

async fn privileged_user_can_manage_settings(admin: bool, owner: bool) {
    let h = common::harness_requiring_login().await;
    let cookie = session(&h, admin, owner).await;
    seed_keys(&h).await;

    let page = h
        .app
        .clone()
        .oneshot(settings_request("GET", "/settings", &cookie, ""))
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let html = common::text(page).await;
    assert!(html.contains("API keys"));
    assert!(!html.contains("stored-openai-secret"));
    assert!(!html.contains("stored-anthropic-secret"));
    assert!(html.contains("A saved key is configured"));

    let response = h
        .app
        .clone()
        .oneshot(settings_request(
            "POST",
            "/settings/ai",
            &cookie,
            "openai_api_key=replacement-secret&anthropic_api_key=",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = common::text(response).await;
    assert!(html.contains("Settings saved."));
    assert!(!html.contains("replacement-secret"));
    assert!(!html.contains("stored-anthropic-secret"));
    let keys = stored_keys(&h).await;
    assert_eq!(keys.openai_api_key, "replacement-secret");
    assert_eq!(keys.anthropic_api_key, "stored-anthropic-secret");

    let response = h
        .app
        .clone()
        .oneshot(settings_request(
            "POST",
            "/settings/mcp",
            &cookie,
            "mcp_enabled=on",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        h.state
            .settings_repo
            .get("mcp.enabled")
            .await
            .unwrap()
            .unwrap()
            .value,
        "true"
    );
}

#[tokio::test]
async fn administrator_can_manage_settings_without_receiving_saved_keys() {
    privileged_user_can_manage_settings(true, false).await;
}

#[tokio::test]
async fn owner_can_manage_settings_without_receiving_saved_keys() {
    privileged_user_can_manage_settings(false, true).await;
}

#[tokio::test]
async fn blank_credential_fields_preserve_saved_keys() {
    let h = common::harness_requiring_login().await;
    let cookie = session(&h, true, false).await;
    seed_keys(&h).await;

    let response = h
        .app
        .clone()
        .oneshot(settings_request(
            "POST",
            "/settings/ai",
            &cookie,
            "openai_api_key=&anthropic_api_key=",
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let keys = stored_keys(&h).await;
    assert_eq!(keys.openai_api_key, "stored-openai-secret");
    assert_eq!(keys.anthropic_api_key, "stored-anthropic-secret");
}

#[tokio::test]
async fn ordinary_user_can_still_open_personal_security_settings() {
    let h = common::harness_requiring_login().await;
    let cookie = session(&h, false, false).await;

    let response = h
        .app
        .clone()
        .oneshot(settings_request("GET", "/settings/security", &cookie, ""))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn explicit_clear_removes_only_the_selected_saved_key() {
    let h = common::harness_requiring_login().await;
    let cookie = session(&h, true, false).await;
    seed_keys(&h).await;

    let response = h
        .app
        .clone()
        .oneshot(settings_request(
            "POST",
            "/settings/ai",
            &cookie,
            "openai_api_key=&anthropic_api_key=&clear_openai_api_key=on",
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let keys = stored_keys(&h).await;
    assert_eq!(keys.openai_api_key, "");
    assert_eq!(keys.anthropic_api_key, "stored-anthropic-secret");
}

#[tokio::test]
async fn administrator_can_save_and_test_local_processing_settings() {
    let h = common::harness_requiring_login().await;
    let cookie = session(&h, true, false).await;
    let form = "ocr_provider=none&embeddings_provider=none";

    let saved = h
        .app
        .clone()
        .oneshot(settings_request(
            "POST",
            "/settings/processing",
            &cookie,
            form,
        ))
        .await
        .unwrap();
    assert_eq!(saved.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        h.state
            .settings_repo
            .get("ocr.provider")
            .await
            .unwrap()
            .unwrap()
            .value,
        "none"
    );
    let tested = h
        .app
        .clone()
        .oneshot(settings_request(
            "POST",
            "/settings/processing/test",
            &cookie,
            form,
        ))
        .await
        .unwrap();
    assert_eq!(tested.status(), StatusCode::OK);
}

#[tokio::test]
async fn administrator_can_request_global_reembedding() {
    let h = common::harness_requiring_login().await;
    let cookie = session(&h, true, false).await;

    let response = h
        .app
        .clone()
        .oneshot(settings_request(
            "POST",
            "/settings/processing/reembed",
            &cookie,
            "",
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/settings?reembedded=0");
}
