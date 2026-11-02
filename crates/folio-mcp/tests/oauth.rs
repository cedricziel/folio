//! `/mcp` behind Folio's OAuth bearer validator.

mod common;

use std::sync::Arc;

use axum::middleware::from_fn_with_state;
use common::Fixture;
use folio_auth::{crypto::hash_token, require_bearer, AccessToken, AuthStore, TokenValidator};
use folio_mcp::McpOptions;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::json;

struct Server {
    url: String,
    store: Arc<dyn AuthStore>,
    vault: folio_core::models::VaultId,
    vault_repo: Arc<dyn folio_core::ports::VaultRepository>,
    members: Arc<dyn folio_core::ports::MembershipRepository>,
    _dir: tempfile::TempDir,
}

async fn serve(fx: &Fixture) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let store = fx.auth.clone();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mcp = folio_mcp::router(
        fx.library.clone(),
        fx.vaults.clone(),
        fx.settings_port(),
        McpOptions {
            enabled_by_default: true,
            ..McpOptions::default()
        },
    )
    .layer(axum::middleware::from_fn_with_state(
        store.clone(),
        folio_mcp::bridge_auth_context,
    ))
    .layer(from_fn_with_state(
        TokenValidator::new(store.clone(), Some(format!("http://{addr}")), "/mcp"),
        require_bearer,
    ));
    tokio::spawn(async move { axum::serve(listener, mcp).await.unwrap() });
    Server {
        url: format!("http://{addr}/mcp"),
        store,
        vault: fx.vault,
        vault_repo: fx.vault_repo.clone(),
        members: fx.members.clone(),
        _dir: dir,
    }
}

async fn issue(server: &Server, token: &str, scopes: &[&str]) {
    server
        .store
        .create_first_user(folio_auth::User {
            id: "u1".into(),
            username: "alice".into(),
            password_hash: "x".into(),
            created_at: 0,
            email: None,
            display_name: None,
            is_admin: true,
            is_owner: false,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    let user = folio_core::models::UserId::from("u1");
    server
        .members
        .grant(
            server.vault,
            &folio_core::models::Principal::User(user.clone()),
            folio_core::models::VaultRole::Owner,
            None,
        )
        .await
        .unwrap();
    server
        .vault_repo
        .set_default(&user, server.vault)
        .await
        .unwrap();
    server.vault_repo.mark_provisioned(&user).await.unwrap();
    server
        .store
        .insert_client(folio_auth::OAuthClient {
            client_id: "c1".into(),
            client_name: "Test".into(),
            redirect_uris: vec!["https://a.example/cb".into()],
            created_at: 0,
        })
        .await
        .unwrap();
    server
        .store
        .insert_access_token(AccessToken {
            token_hash: hash_token(token),
            client_id: "c1".into(),
            user_id: "u1".into(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            audience: server.url.clone(),
            expires_at: i64::MAX / 2,
            grant_id: "g1".into(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn missing_token_gets_401_with_resource_metadata() {
    let fx = Fixture::new().await;
    let server = serve(&fx).await;
    let res = reqwest::Client::new()
        .post(&server.url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
    let challenge = res.headers()["www-authenticate"].to_str().unwrap();
    assert!(challenge.starts_with("Bearer resource_metadata=\""));
    assert!(challenge.ends_with("/.well-known/oauth-protected-resource\""));
}

#[tokio::test]
async fn read_token_can_search_but_not_update() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Invoice", "total due").await;
    let server = serve(&fx).await;
    issue(&server, "read-token", &["documents:read"]).await;

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(server.url.clone()).auth_header("read-token"),
    );
    let client = ().serve(transport).await.unwrap();

    let read = client
        .call_tool(
            CallToolRequestParams::new("search_documents")
                .with_arguments(json!({"query": "invoice"}).as_object().cloned().unwrap()),
        )
        .await
        .unwrap();
    assert_ne!(read.is_error, Some(true));

    let write = client
        .call_tool(
            CallToolRequestParams::new("update_document").with_arguments(
                json!({"id": doc.id.to_string(), "title": "Hacked"})
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await;
    let denied = match write {
        Err(e) => e.to_string().contains("insufficient scope"),
        Ok(r) => {
            r.is_error == Some(true) && format!("{:?}", r.content).contains("insufficient scope")
        }
    };
    assert!(denied, "write tool must require documents:write");
    assert_eq!(
        fx.library
            .get_document(fx.vault, doc.id)
            .await
            .unwrap()
            .document
            .title,
        "Invoice"
    );
}
