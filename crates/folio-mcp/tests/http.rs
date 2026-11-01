mod common;

use std::collections::HashSet;

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use common::Fixture;
use folio_mcp::{McpAuth, McpOptions};
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::json;

async fn serve(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/mcp")
}

async fn inject_auth(
    State(scopes): State<&'static [&'static str]>,
    mut req: Request,
    next: Next,
) -> Response {
    req.extensions_mut().insert(McpAuth::new(
        Some("alice".into()),
        scopes.iter().map(|s| s.to_string()).collect::<HashSet<_>>(),
    ));
    next.run(req).await
}

fn with_scopes(app: axum::Router, scopes: &'static [&'static str]) -> axum::Router {
    app.layer(axum::middleware::from_fn_with_state(scopes, inject_auth))
}

#[tokio::test]
async fn read_scope_allows_reads_but_not_writes() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Invoice", "total due").await;
    let app = with_scopes(
        folio_mcp::router(
            fx.library.clone(),
            fx.settings_port(),
            McpOptions::default(),
        ),
        &["documents:read"],
    );
    let url = serve(app).await;

    let client = ().serve(StreamableHttpClientTransport::from_uri(url)).await.unwrap();

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
    let unchanged = fx.library.get_document(doc.id).await.unwrap();
    assert_eq!(unchanged.document.title, "Invoice");
}

#[tokio::test]
async fn write_scope_allows_writes() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Invoice", "total due").await;
    let app = with_scopes(
        folio_mcp::router(
            fx.library.clone(),
            fx.settings_port(),
            McpOptions::default(),
        ),
        &["documents:read", "documents:write"],
    );
    let url = serve(app).await;
    let client = ().serve(StreamableHttpClientTransport::from_uri(url)).await.unwrap();

    let res = client
        .call_tool(
            CallToolRequestParams::new("update_document").with_arguments(
                json!({"id": doc.id.to_string(), "title": "Paid"})
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(res.is_error, Some(true));
    assert_eq!(
        fx.library
            .get_document(doc.id)
            .await
            .unwrap()
            .document
            .title,
        "Paid"
    );
}

#[tokio::test]
async fn disabled_setting_hides_endpoint() {
    let fx = Fixture::new().await;
    let app = folio_mcp::router(
        fx.library.clone(),
        fx.settings_port(),
        McpOptions::default(),
    );
    let url = serve(app).await;
    let http = reqwest::Client::new();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;
    let post = || {
        http.post(&url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(init)
            .send()
    };

    assert!(post().await.unwrap().status().is_success());

    use folio_core::ports::SettingsRepository;
    fx.settings.set("mcp.enabled", "false").await.unwrap();
    assert_eq!(post().await.unwrap().status(), 404);

    let off_by_default = McpOptions {
        enabled_by_default: false,
        ..McpOptions::default()
    };
    let url = serve(folio_mcp::router(
        fx.library.clone(),
        fx.settings_port(),
        off_by_default,
    ))
    .await;
    fx.settings.set("mcp.enabled", "true").await.unwrap();
    let resp = http
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(init)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "DB setting overrides config default"
    );
}
