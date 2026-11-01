mod common;

use base64::Engine;
use common::{call, call_ok, Fixture};
use serde_json::json;

#[tokio::test]
async fn search_finds_inserted_document() {
    let fx = Fixture::new().await;
    let doc = fx
        .insert("Electricity invoice", "Total due: 100 EUR for March")
        .await;
    fx.insert("Birthday card", "Happy birthday!").await;
    let client = fx.in_process_client().await;

    let out = call_ok(&client, "search_documents", json!({"query": "total due"})).await;

    let results = out["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["id"], doc.id.to_string());
    assert_eq!(results[0]["title"], "Electricity invoice");
    assert!(results[0]["snippet"]
        .as_str()
        .unwrap()
        .contains("Total due"));
}

#[tokio::test]
async fn search_filters_by_tag_and_rejects_unknown_tag() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Invoice", "pay soon").await;
    fx.insert("Other invoice", "pay later").await;
    let client = fx.in_process_client().await;
    call_ok(
        &client,
        "add_tags",
        json!({"id": doc.id.to_string(), "tags": ["tax"]}),
    )
    .await;

    let out = call_ok(
        &client,
        "search_documents",
        json!({"query": "invoice", "tags": ["TAX"]}),
    )
    .await;
    assert_eq!(out["results"].as_array().unwrap().len(), 1);
    assert_eq!(out["results"][0]["id"], doc.id.to_string());

    let err = call(&client, "search_documents", json!({"tags": ["nope"]}))
        .await
        .map(|r| r.is_error)
        .map_err(|_| ());
    assert!(matches!(err, Err(()) | Ok(Some(true))));
}

#[tokio::test]
async fn update_document_changes_fields() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Scan 001", "some text").await;
    let client = fx.in_process_client().await;
    let id = doc.id.to_string();

    call_ok(
        &client,
        "update_document",
        json!({
            "id": id,
            "title": "Rent contract",
            "correspondent": "ACME Housing",
            "document_type": "Contract",
            "tags": ["home", "legal"]
        }),
    )
    .await;

    let out = call_ok(&client, "get_document", json!({"id": id})).await;
    assert_eq!(out["title"], "Rent contract");
    assert_eq!(out["correspondent"], "ACME Housing");
    assert_eq!(out["document_type"], "Contract");
    let mut tags: Vec<&str> = out["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    tags.sort();
    assert_eq!(tags, ["home", "legal"]);

    call_ok(&client, "remove_tags", json!({"id": id, "tags": ["legal"]})).await;
    call_ok(
        &client,
        "update_document",
        json!({"id": id, "correspondent": ""}),
    )
    .await;
    let out = call_ok(&client, "get_document", json!({"id": id})).await;
    assert_eq!(out["tags"], json!(["home"]));
    assert_eq!(out["correspondent"], json!(null));
}

#[tokio::test]
async fn get_document_paginates_long_text() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Long", &"abcdefghij".repeat(10)).await;
    let client = fx.in_process_client().await;
    let id = doc.id.to_string();

    let first = call_ok(&client, "get_document", json!({"id": id, "max_chars": 60})).await;
    assert_eq!(first["text"].as_str().unwrap().len(), 60);
    assert_eq!(first["total_chars"], 100);
    let cursor = first["next_cursor"].as_str().unwrap().to_string();

    let second = call_ok(
        &client,
        "get_document",
        json!({"id": id, "max_chars": 60, "cursor": cursor}),
    )
    .await;
    assert_eq!(second["text"].as_str().unwrap().len(), 40);
    assert_eq!(second["next_cursor"], json!(null));
}

#[tokio::test]
async fn upload_returns_id_and_pending_status() {
    let fx = Fixture::new().await;
    let client = fx.in_process_client().await;
    let content = base64::engine::general_purpose::STANDARD.encode(b"hello world");

    let out = call_ok(
        &client,
        "upload_document",
        json!({"filename": "note.txt", "content_base64": content}),
    )
    .await;
    let id = out["id"].as_str().unwrap().to_string();
    assert_eq!(out["processing_status"], "pending");

    let status = call_ok(&client, "get_processing_status", json!({"id": id})).await;
    assert_eq!(status["status"], "pending");

    let out = call_ok(&client, "get_document", json!({"id": id})).await;
    assert_eq!(out["title"], "note.txt");
}

#[tokio::test]
async fn listing_tools_return_vocabulary() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Doc", "x").await;
    let client = fx.in_process_client().await;
    call_ok(
        &client,
        "update_document",
        json!({"id": doc.id.to_string(), "correspondent": "Bank", "document_type": "Statement", "tags": ["money"]}),
    )
    .await;

    let tags = call_ok(&client, "list_tags", json!({})).await;
    assert_eq!(tags["tags"][0]["name"], "money");
    let corrs = call_ok(&client, "list_correspondents", json!({})).await;
    assert_eq!(corrs["correspondents"][0]["name"], "Bank");
    let types = call_ok(&client, "list_document_types", json!({})).await;
    assert_eq!(types["document_types"][0]["name"], "Statement");
}

#[tokio::test]
async fn tools_carry_safety_annotations_and_none_delete() {
    let fx = Fixture::new().await;
    let client = fx.in_process_client().await;
    let tools = client.list_all_tools().await.unwrap();

    assert!(tools.iter().all(|t| !t.name.contains("delete")));
    let hints = |name: &str| {
        tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("missing tool {name}"))
            .annotations
            .clone()
            .unwrap()
    };
    assert_eq!(hints("search_documents").read_only_hint, Some(true));
    assert_eq!(hints("get_document").read_only_hint, Some(true));
    let update = hints("update_document");
    assert_eq!(update.read_only_hint, Some(false));
    assert_eq!(update.idempotent_hint, Some(true));
    assert_eq!(hints("add_tags").destructive_hint, Some(false));
    assert_eq!(hints("upload_document").idempotent_hint, Some(false));
}

#[tokio::test]
async fn resources_expose_document_text_and_file() {
    let fx = Fixture::new().await;
    let client = fx.in_process_client().await;
    let content = base64::engine::general_purpose::STANDARD.encode(b"hello world");
    let out = call_ok(
        &client,
        "upload_document",
        json!({"filename": "note.txt", "content_base64": content}),
    )
    .await;
    let id = out["id"].as_str().unwrap();

    let templates = client.list_resource_templates(None).await.unwrap();
    assert!(templates
        .resource_templates
        .iter()
        .any(|t| t.uri_template == "folio://documents/{id}"));

    let text = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(format!(
            "folio://documents/{id}"
        )))
        .await
        .unwrap();
    let text = serde_json::to_string(&text.contents).unwrap();
    assert!(text.contains("note.txt"));

    let file = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(format!(
            "folio://documents/{id}/file"
        )))
        .await
        .unwrap();
    let file = serde_json::to_value(&file.contents).unwrap();
    assert_eq!(
        file[0]["blob"],
        base64::engine::general_purpose::STANDARD.encode(b"hello world")
    );
}
