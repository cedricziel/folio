mod common;

use common::Fixture;
use rmcp::model::{ClientRequest, CustomRequest, ReadResourceRequestParams};
use rmcp::service::{ClientLifecycleMode, ClientServiceExt, RunningService};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};

async fn discover_client(fx: &Fixture) -> RunningService<RoleClient, ()> {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    let server = folio_mcp::FolioMcp::new(fx.library.clone(), fx.vaults.clone());
    tokio::spawn(async move {
        server
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    ().serve_with_lifecycle(
        client_io,
        ClientLifecycleMode::Discover {
            preferred_versions: vec![rmcp::model::ProtocolVersion::V_2026_07_28],
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn skills_are_discoverable_and_directly_retrievable() {
    let fx = Fixture::new().await;
    let client = discover_client(&fx).await;
    let info = serde_json::to_value(client.peer_info().unwrap()).unwrap();
    assert_eq!(
        info["capabilities"]["extensions"]["io.modelcontextprotocol/skills"],
        json!({})
    );
    let get = client
        .send_request(ClientRequest::CustomRequest(CustomRequest::new(
            "skills/get",
            Some(json!({"uri": "skill://folio-archive/SKILL.md"})),
        )))
        .await
        .unwrap();
    let get = serde_json::to_value(get).unwrap();
    let list = client
        .send_request(ClientRequest::CustomRequest(CustomRequest::new(
            "skills/list",
            Some(json!({})),
        )))
        .await
        .unwrap();
    let list = serde_json::to_value(list).unwrap();
    assert_eq!(list["skills"], json!([get["skill"]]));
    assert_eq!(list["resultType"], "complete");
    assert_eq!(list["cacheScope"], "public");
    assert!(list["ttlMs"].as_u64().unwrap() > 0);
    assert_eq!(get["skill"]["frontmatter"]["name"], "folio-archive");
}

#[tokio::test]
async fn unknown_skill_files_are_invalid_params() {
    let fx = Fixture::new().await;
    let client = discover_client(&fx).await;
    let error = client
        .read_resource(ReadResourceRequestParams::new(
            "skill://folio-archive/../secret",
        ))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("-32602"), "{error}");
}

#[tokio::test]
async fn invalid_skill_requests_are_rejected() {
    let fx = Fixture::new().await;
    let client = discover_client(&fx).await;
    for (method, params) in [
        ("skills/get", json!({"uri": "skill://missing/SKILL.md"})),
        ("skills/get", json!({})),
        ("skills/get", json!({"uri": 42})),
        ("skills/list", json!({"cursor": "invalid"})),
    ] {
        let error = client
            .send_request(ClientRequest::CustomRequest(CustomRequest::new(
                method,
                Some(params),
            )))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("-32602"), "{error}");
    }
}

#[tokio::test]
async fn manifest_matches_every_served_file_and_frontmatter() {
    use sha2::{Digest, Sha256};
    let fx = Fixture::new().await;
    let client = discover_client(&fx).await;
    let result = client
        .send_request(ClientRequest::CustomRequest(CustomRequest::new(
            "skills/get",
            Some(json!({"uri": "skill://folio-archive/SKILL.md"})),
        )))
        .await
        .unwrap();
    let result = serde_json::to_value(result).unwrap();
    let skill = &result["skill"];
    let files = skill["resources"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    for file in files {
        let uri = file["uri"].as_str().unwrap();
        let read = client
            .read_resource(ReadResourceRequestParams::new(uri))
            .await
            .unwrap();
        let read = serde_json::to_value(read).unwrap();
        let text = read["contents"][0]["text"].as_str().unwrap();
        assert_eq!(read["contents"][0]["uri"], uri);
        assert_eq!(read["contents"][0]["mimeType"], "text/markdown");
        assert_eq!(file["size"], text.len());
        assert_eq!(
            file["digest"],
            format!(
                "sha256:{}",
                Sha256::digest(text.as_bytes())
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            )
        );
        assert_eq!(read["cacheScope"], "public");
        assert!(read["ttlMs"].as_u64().unwrap() > 0);
    }
    let read = client
        .read_resource(ReadResourceRequestParams::new(
            "skill://folio-archive/SKILL.md",
        ))
        .await
        .unwrap();
    let read = serde_json::to_value(read).unwrap();
    let text = read["contents"][0]["text"].as_str().unwrap();
    let frontmatter: Value = serde_json::from_str(
        text.strip_prefix("---\n")
            .unwrap()
            .split_once("\n---\n")
            .unwrap()
            .0,
    )
    .unwrap();
    assert_eq!(frontmatter, skill["frontmatter"]);
}

#[tokio::test]
async fn resources_read_returns_skill_content() {
    let fx = Fixture::new().await;
    let client = fx.in_process_client().await;
    let read = client
        .read_resource(ReadResourceRequestParams::new(
            "skill://folio-archive/SKILL.md",
        ))
        .await
        .unwrap();
    let read = serde_json::to_value(read).unwrap();
    assert!(read["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("# Use the Folio archive"));
}
