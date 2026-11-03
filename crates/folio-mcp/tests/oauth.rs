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
    serve_with_required(fx, true).await
}

async fn serve_with_required(fx: &Fixture, required: bool) -> Server {
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
        TokenValidator::new(store.clone(), Some(format!("http://{addr}")), "/mcp")
            .with_required(required),
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
        .create_user(folio_auth::User {
            id: "other-owner".into(),
            username: "other-owner".into(),
            password_hash: "!".into(),
            created_at: 0,
            email: None,
            display_name: None,
            is_admin: false,
            is_owner: false,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    server
        .members
        .grant(
            server.vault,
            &folio_core::models::Principal::User("other-owner".into()),
            folio_core::models::VaultRole::Owner,
            None,
        )
        .await
        .unwrap();
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
    assert!(challenge.ends_with("/.well-known/oauth-protected-resource/mcp\""));
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

#[tokio::test]
async fn personal_token_revocation_is_checked_after_mcp_initialization() {
    let fx = Fixture::new().await;
    let server = serve(&fx).await;
    issue(&server, "oauth-token", &["documents:read"]).await;
    server
        .store
        .insert_personal_token(folio_auth::PersonalToken {
            id: "pat".into(),
            user_id: "u1".into(),
            name: "MCP".into(),
            token_hash: hash_token("folio_pat_test"),
            prefix: "folio_pat_test".into(),
            audience: server.url.clone(),
            scopes: vec!["documents:read".into()],
            created_at: 1,
            expires_at: i64::MAX,
            revoked_at: None,
        })
        .await
        .unwrap();
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(server.url.clone())
            .auth_header("folio_pat_test"),
    );
    let client = ().serve(transport).await.unwrap();
    let allowed = client
        .call_tool(CallToolRequestParams::new("list_tags"))
        .await
        .unwrap();
    assert_ne!(allowed.is_error, Some(true));
    let session = legacy_session(&server, "folio_pat_test").await;
    server
        .store
        .revoke_personal_token("u1", "pat", 2)
        .await
        .unwrap();
    assert!(client
        .call_tool(CallToolRequestParams::new("list_tags"))
        .await
        .is_err());
    assert_eq!(
        session_request(&server, reqwest::Method::GET, &session, "folio_pat_test")
            .await
            .status(),
        401
    );
    let _ = client.cancel().await;
}

#[tokio::test]
async fn live_vault_role_prevents_viewer_mutation_with_write_scope() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Invoice", "total due").await;
    let server = serve(&fx).await;
    issue(
        &server,
        "write-token",
        &["documents:read", "documents:write"],
    )
    .await;
    server
        .members
        .grant(
            server.vault,
            &folio_core::models::Principal::User("u1".into()),
            folio_core::models::VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(server.url.clone())
            .auth_header("write-token"),
    );
    let client = ().serve(transport).await.unwrap();
    let denied = client
        .call_tool(
            CallToolRequestParams::new("update_document").with_arguments(
                json!({"id":doc.id,"title":"Forbidden"})
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(denied.is_error, Some(true));
    assert_eq!(
        fx.library
            .get_document(fx.vault, doc.id)
            .await
            .unwrap()
            .document
            .title,
        "Invoice"
    );
    let _ = client.cancel().await;
}

async fn legacy_session(server: &Server, token: &str) -> String {
    let http = reqwest::Client::new();
    let request = http.post(&server.url);
    let request = if token.is_empty() {
        request
    } else {
        request.bearer_auth(token)
    };
    let response = request.header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"session-test","version":"1"}}}))
        .send().await.unwrap();
    assert!(response.status().is_success());
    response.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_string()
}

async fn session_request(
    server: &Server,
    method: reqwest::Method,
    session: &str,
    token: &str,
) -> reqwest::Response {
    let http = reqwest::Client::new();
    let request = http.request(method, &server.url);
    let request = if token.is_empty() {
        request
    } else {
        request.bearer_auth(token)
    };
    request
        .header("mcp-session-id", session)
        .header("mcp-protocol-version", "2025-06-18")
        .header("accept", "application/json, text/event-stream")
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn legacy_sessions_bind_identity_allow_oauth_rotation_and_reject_changed_vault_roles() {
    let fx = Fixture::new().await;
    let server = serve(&fx).await;
    issue(&server, "original-token", &["documents:read"]).await;
    server
        .store
        .insert_access_token(AccessToken {
            token_hash: hash_token("replacement-token"),
            client_id: "c1".into(),
            user_id: "u1".into(),
            scopes: vec!["documents:read".into()],
            audience: server.url.clone(),
            expires_at: i64::MAX,
            grant_id: "g1".into(),
        })
        .await
        .unwrap();
    server
        .store
        .insert_personal_token(folio_auth::PersonalToken {
            id: "different-credential".into(),
            user_id: "u1".into(),
            name: "PAT".into(),
            token_hash: hash_token("folio_pat_other"),
            prefix: "folio_pat_other".into(),
            audience: server.url.clone(),
            scopes: vec!["documents:read".into()],
            created_at: 1,
            expires_at: i64::MAX,
            revoked_at: None,
        })
        .await
        .unwrap();
    let session = legacy_session(&server, "original-token").await;
    assert_eq!(
        session_request(&server, reqwest::Method::GET, &session, "folio_pat_other")
            .await
            .status(),
        404
    );
    assert_eq!(
        session_request(
            &server,
            reqwest::Method::DELETE,
            &session,
            "folio_pat_other"
        )
        .await
        .status(),
        404
    );
    let response =
        session_request(&server, reqwest::Method::GET, &session, "replacement-token").await;
    assert!(
        response.status().is_success(),
        "OAuth rotation keeps the session"
    );
    drop(response);
    server
        .members
        .grant(
            server.vault,
            &folio_core::models::Principal::User("u1".into()),
            folio_core::models::VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        session_request(&server, reqwest::Method::GET, &session, "replacement-token")
            .await
            .status(),
        404
    );
    assert_eq!(
        session_request(
            &server,
            reqwest::Method::DELETE,
            &session,
            "replacement-token"
        )
        .await
        .status(),
        404
    );
}

#[tokio::test]
async fn optional_auth_distinguishes_anonymous_and_authenticated_legacy_sessions() {
    let fx = Fixture::new().await;
    let server = serve_with_required(&fx, false).await;
    issue(&server, "user-token", &["documents:read"]).await;
    let anonymous = legacy_session(&server, "").await;
    assert_eq!(
        session_request(&server, reqwest::Method::GET, &anonymous, "user-token")
            .await
            .status(),
        404
    );
    let authenticated = legacy_session(&server, "user-token").await;
    assert_eq!(
        session_request(&server, reqwest::Method::GET, &authenticated, "")
            .await
            .status(),
        404
    );
    assert_eq!(
        session_request(&server, reqwest::Method::GET, &authenticated, "invalid")
            .await
            .status(),
        401
    );
}

#[tokio::test]
async fn contributors_edit_own_documents_but_cannot_create_vocabulary_or_edit_others() {
    use folio_core::models::{CreateDocument, Principal, UserId, VaultRole};
    let fx = Fixture::new().await;
    let other = fx.insert("Other", "private").await;
    let server = serve(&fx).await;
    issue(
        &server,
        "contributor-token",
        &["documents:read", "documents:write"],
    )
    .await;
    let user = UserId::from("u1");
    let own = fx
        .documents
        .create(
            server.vault,
            CreateDocument {
                title: "Own".into(),
                uploaded_by: Some(user.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    server
        .members
        .grant(
            server.vault,
            &Principal::User(user),
            VaultRole::Contributor,
            None,
        )
        .await
        .unwrap();
    let client = ()
        .serve(StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(server.url.clone())
                .auth_header("contributor-token"),
        ))
        .await
        .unwrap();
    let renamed = client
        .call_tool(
            CallToolRequestParams::new("update_document").with_arguments(
                json!({"id":own.id,"title":"Renamed"})
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(renamed.is_error, Some(true));
    let new_tag = client
        .call_tool(
            CallToolRequestParams::new("add_tags").with_arguments(
                json!({"id":own.id,"tags":["forbidden-new-tag"]})
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(new_tag.is_error, Some(true));
    assert!(fx.library.list_tags(server.vault).await.unwrap().is_empty());
    let other_edit = client
        .call_tool(
            CallToolRequestParams::new("update_document").with_arguments(
                json!({"id":other.id,"title":"Denied"})
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(other_edit.is_error, Some(true));
    let _ = client.cancel().await;
}

#[tokio::test]
async fn contributor_cannot_recreate_a_tag_deleted_after_vocabulary_read() {
    assert_deleted_tag_is_not_recreated("add_tags").await;
}

#[tokio::test]
async fn contributor_metadata_cannot_recreate_a_tag_deleted_after_vocabulary_read() {
    assert_deleted_tag_is_not_recreated("update_document").await;
}

async fn assert_deleted_tag_is_not_recreated(tool: &'static str) {
    use folio_core::models::{CreateDocument, Principal, UserId, VaultRole};

    let fx = Fixture::new().await;
    let server = serve(&fx).await;
    issue(&server, "contributor-token", &["documents:write"]).await;
    let user = UserId::from("u1");
    let document = fx
        .documents
        .create(
            server.vault,
            CreateDocument {
                title: "Own document".into(),
                uploaded_by: Some(user.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let tagged_elsewhere = fx.insert("Vocabulary source", "").await;
    fx.library
        .add_tags(
            server.vault,
            tagged_elsewhere.id,
            &["existing".into()],
            folio_core::services::VocabularyPolicy::CreateMissing,
        )
        .await
        .unwrap();
    server
        .members
        .grant(
            server.vault,
            &Principal::User(user),
            VaultRole::Contributor,
            None,
        )
        .await
        .unwrap();
    let client = ()
        .serve(StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(server.url.clone())
                .auth_header("contributor-token"),
        ))
        .await
        .unwrap();
    fx.delete_tags_after_next_list
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let result = client
        .call_tool(
            CallToolRequestParams::new(tool).with_arguments(
                json!({"id":document.id,"tags":["existing"]})
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap();

    assert_eq!(
        result.is_error,
        Some(true),
        "a Contributor must not recreate concurrently deleted vocabulary"
    );
    assert!(fx.library.list_tags(server.vault).await.unwrap().is_empty());
    let _ = client.cancel().await;
}

#[tokio::test]
async fn mutation_actor_comes_from_bearer_auth_instead_of_tool_arguments() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Invoice", "total due").await;
    let server = serve(&fx).await;
    issue(
        &server,
        "actor-token",
        &["documents:read", "documents:write"],
    )
    .await;
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(server.url.clone())
            .auth_header("actor-token"),
    );
    let client = ().serve(transport).await.unwrap();
    common::call_ok(
        &client,
        "add_tags",
        json!({
            "id": doc.id.to_string(), "tags": ["actor audit"],
            "actor": "forged", "created_by": "forged"
        }),
    )
    .await;
    assert_eq!(
        *fx.tag_actors.lock().unwrap(),
        vec![Some(folio_core::models::UserId::from("u1"))]
    );
    assert_eq!(folio_core::mutation::current_actor(), None);
}

#[tokio::test]
async fn personal_token_mutations_record_the_token_owner_as_actor() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Invoice", "total due").await;
    let server = serve(&fx).await;
    issue(&server, "setup-token", &["documents:read"]).await;
    server
        .store
        .insert_personal_token(folio_auth::PersonalToken {
            id: "actor-pat".into(),
            user_id: "u1".into(),
            name: "actor test".into(),
            token_hash: hash_token("folio_pat_actor"),
            prefix: "folio_pat_actor".into(),
            audience: server.url.clone(),
            scopes: vec!["documents:read".into(), "documents:write".into()],
            created_at: 1,
            expires_at: i64::MAX / 2,
            revoked_at: None,
        })
        .await
        .unwrap();
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(server.url.clone())
            .auth_header("folio_pat_actor"),
    );
    let client = ().serve(transport).await.unwrap();
    common::call_ok(
        &client,
        "add_tags",
        json!({
            "id": doc.id.to_string(), "tags": ["token actor"], "actor": "forged"
        }),
    )
    .await;
    assert_eq!(
        *fx.tag_actors.lock().unwrap(),
        vec![Some(folio_core::models::UserId::from("u1"))]
    );
}
