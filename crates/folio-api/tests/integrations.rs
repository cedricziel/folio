mod common;

use axum::http::{header, StatusCode};
use common::{get, harness, text};
use tower::ServiceExt;

#[tokio::test]
async fn integration_shortcuts_are_global_while_browsing_a_vault() {
    let h = harness().await;
    let body = text(
        h.app
            .clone()
            .oneshot(get(&format!("/vaults/{}/", h.vault)))
            .await
            .unwrap(),
    )
    .await;
    assert!(body.contains("href=\"/api/docs\""));
    assert!(body.contains("href=\"/settings/integrations\""));
    assert!(!body.contains(&format!("/vaults/{}/api/docs", h.vault)));
}

#[tokio::test]
async fn integrations_uses_configured_url_and_requires_sign_in_for_tokens() {
    let h = harness().await;
    let mut state = h.state.clone().with_mcp_default(true);
    state.public_url = "https://archive.example.test/".into();
    let app = folio_api::create_router(
        state,
        folio_api::AuthState::new(h.auth_store.clone(), None).with_required(false),
    );
    let mut request = get("/settings/integrations");
    request
        .headers_mut()
        .insert(header::HOST, "untrusted.example.test".parse().unwrap());
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = text(response).await;
    assert!(body.contains("https://archive.example.test/mcp"));
    assert!(!body.contains("untrusted.example.test"));
    assert!(body.contains("Sign in to manage personal access tokens"));
    assert!(!body.contains("id=\"token-create\""));
}

#[tokio::test]
async fn integrations_database_setting_overrides_mcp_default() {
    let h = harness().await;
    let app = folio_api::create_router(
        h.state.clone().with_mcp_default(true),
        folio_api::AuthState::new(h.auth_store.clone(), None).with_required(false),
    );
    h.state
        .settings_repo
        .set("mcp.enabled", "false")
        .await
        .unwrap();
    let body = text(
        app.clone()
            .oneshot(get("/settings/integrations"))
            .await
            .unwrap(),
    )
    .await;
    assert!(body.contains("MCP server is disabled"));
    assert!(!body.contains("id=\"mcp-url\""));
    h.state
        .settings_repo
        .set("mcp.enabled", " TrUe ")
        .await
        .unwrap();
    let body = text(
        app.clone()
            .oneshot(get("/settings/integrations"))
            .await
            .unwrap(),
    )
    .await;
    assert!(body.contains("id=\"mcp-url\""));
}

#[tokio::test]
async fn optional_auth_signed_in_users_can_manage_integration_tokens() {
    let h = harness().await;
    let id = uuid::Uuid::new_v4().to_string();
    h.auth_store
        .create_user(folio_auth::User {
            id: id.clone(),
            username: "ada".into(),
            password_hash: "!".into(),
            created_at: 1,
            email: None,
            display_name: None,
            is_admin: false,
            is_owner: false,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    h.auth_store
        .create_session(folio_auth::Session {
            id_hash: folio_auth::crypto::hash_token("integration-session"),
            user_id: id.clone(),
            expires_at: i64::MAX,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    h.state
        .vaults
        .provision_user(&folio_core::models::UserId(id))
        .await
        .unwrap();
    let mut request = get("/settings/integrations");
    request.headers_mut().insert(
        header::COOKIE,
        "folio_session=integration-session".parse().unwrap(),
    );
    let response = h.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(body.contains("id=\"token-create\""));
    assert!(body.contains("value=\"documents:read\" checked"));
    assert!(!body.contains("value=\"documents:write\" checked"));
    assert!(body.contains("value=\"30\" required"));
    assert!(!body.contains("Sign in to manage personal access tokens"));
}
