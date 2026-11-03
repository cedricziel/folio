mod common;

use base64::Engine;
use common::{call, call_ok, FakeEmbeddings, FakeSearch, Fixture};
use serde_json::json;
use std::sync::Arc;

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
async fn search_reads_the_tag_table_once_however_many_tags_are_named() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Invoice", "pay soon").await;
    let client = fx.in_process_client().await;
    call_ok(
        &client,
        "add_tags",
        json!({"id": doc.id.to_string(), "tags": ["a", "b", "c"]}),
    )
    .await;
    fx.tag_lists.store(0, std::sync::atomic::Ordering::SeqCst);

    let out = call_ok(
        &client,
        "search_documents",
        json!({"query": "invoice", "tags": ["a", "b", "c"]}),
    )
    .await;

    assert_eq!(out["results"].as_array().unwrap().len(), 1);
    assert_eq!(fx.tag_lists.load(std::sync::atomic::Ordering::SeqCst), 1);
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

    let events = fx
        .outbox
        .replay(0, chrono::Utc::now() - chrono::Duration::days(1), 100)
        .await
        .unwrap();
    let title_event = events
        .iter()
        .filter_map(|event| event.envelope.as_ref())
        .find(|event| event.event_type() == "document.title_changed")
        .unwrap();
    assert_eq!(title_event.document_id(), Some(doc.id));
    assert_eq!(title_event.vault_id(), fx.vault);
    assert_eq!(title_event.data()["before"], "Scan 001");
    assert_eq!(title_event.data()["after"], "Rent contract");

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

    let stored = fx
        .documents
        .find_by_id(fx.vault, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.source.kind, folio_core::models::SourceKind::Mcp);
}

#[tokio::test]
async fn failed_upload_leaves_no_document_behind() {
    let fx = Fixture::with_failing_store().await;
    let client = fx.in_process_client().await;
    let content = base64::engine::general_purpose::STANDARD.encode(b"hello world");

    let res = call(
        &client,
        "upload_document",
        json!({"filename": "note.txt", "content_base64": content}),
    )
    .await;

    assert!(!matches!(res, Ok(r) if r.is_error != Some(true)));
    assert!(fx.documents.list(fx.vault, None).await.unwrap().is_empty());
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

#[tokio::test]
async fn oversized_file_resource_suggests_text_resource() {
    let fx = Fixture::new().await;
    let (document, _) = fx
        .library
        .upload(
            fx.vault,
            folio_core::services::NewUpload {
                filename: "large.txt".into(),
                title: None,
                mime_type: Some("text/plain".into()),
                data: vec![b'x'; 5 * 1024 * 1024 + 1],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let client = fx.in_process_client().await;

    let error = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(format!(
            "folio://documents/{}/file",
            document.id
        )))
        .await
        .unwrap_err();

    let rmcp::service::ServiceError::McpError(error) = error else {
        panic!("expected a protocol error");
    };
    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(error.message.contains("5242880"));
    assert!(error.message.contains("use the text resource instead"));
}

#[tokio::test]
async fn literal_matches_rank_before_semantic_hits() {
    let search = Arc::new(FakeSearch::default());
    let fx = Fixture::with_semantic(Arc::new(FakeEmbeddings), Some(search.clone())).await;
    let literal = fx.insert("Invoice March", "pay by friday").await;
    let semantic_a = fx.insert("Receipt", "unrelated a").await;
    let semantic_b = fx.insert("Statement", "unrelated b").await;
    search.rank(vec![
        (semantic_a.id, 0.9),
        (semantic_b.id, 0.8),
        (literal.id, 0.1),
    ]);
    let client = fx.in_process_client().await;

    let out = call_ok(
        &client,
        "search_documents",
        json!({"query": "invoice", "limit": 1}),
    )
    .await;

    let results = out["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["id"], literal.id.to_string());
}

#[tokio::test]
async fn search_tag_filter_ignores_name_case_duplicates() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Invoice", "pay soon").await;
    let client = fx.in_process_client().await;
    call_ok(
        &client,
        "add_tags",
        json!({"id": doc.id.to_string(), "tags": ["Invoice"]}),
    )
    .await;

    let out = call_ok(
        &client,
        "search_documents",
        json!({"tags": ["Invoice", "invoice"]}),
    )
    .await;

    assert_eq!(out["count"], 1);
}

#[tokio::test]
async fn search_date_to_includes_the_whole_day() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Today", "x").await;
    let client = fx.in_process_client().await;
    let today = doc.created_at.format("%Y-%m-%d").to_string();

    let out = call_ok(
        &client,
        "search_documents",
        json!({"date_from": today, "date_to": today}),
    )
    .await;

    assert_eq!(out["count"], 1);
}

#[tokio::test]
async fn add_tags_with_an_invalid_name_changes_nothing() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Doc", "x").await;
    let client = fx.in_process_client().await;

    let res = call(
        &client,
        "add_tags",
        json!({"id": doc.id.to_string(), "tags": ["good", " "]}),
    )
    .await;

    assert!(res.is_err() || res.unwrap().is_error == Some(true));
    let tags = call_ok(&client, "list_tags", json!({})).await;
    assert_eq!(tags["tags"], json!([]));
}

#[tokio::test]
async fn update_document_with_an_invalid_tag_changes_nothing() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Doc", "x").await;
    let client = fx.in_process_client().await;

    let res = call(
        &client,
        "update_document",
        json!({"id": doc.id.to_string(), "title": "New", "correspondent": "Acme", "tags": ["good", ""]}),
    )
    .await;

    assert!(res.is_err() || res.unwrap().is_error == Some(true));
    let out = call_ok(&client, "get_document", json!({"id": doc.id.to_string()})).await;
    assert_eq!(out["title"], "Doc");
    assert_eq!(out["correspondent"], json!(null));
    let corrs = call_ok(&client, "list_correspondents", json!({})).await;
    assert_eq!(corrs["correspondents"], json!([]));
}

#[tokio::test]
async fn remove_tags_ignores_unknown_names() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Doc", "x").await;
    let client = fx.in_process_client().await;
    let id = doc.id.to_string();
    call_ok(&client, "add_tags", json!({"id": id, "tags": ["keep"]})).await;

    let out = call_ok(
        &client,
        "remove_tags",
        json!({"id": id, "tags": ["never-existed"]}),
    )
    .await;

    assert_eq!(out["tags"], json!(["keep"]));
}

#[tokio::test]
async fn remove_tags_detaches_a_retained_deleted_tag() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Doc", "x").await;
    let client = fx.in_process_client().await;
    let id = doc.id.to_string();
    call_ok(&client, "add_tags", json!({"id": id, "tags": ["retired"]})).await;
    let retired = fx.documents.get_tags(fx.vault, doc.id).await.unwrap()[0].id;
    fx.tags.delete(fx.vault, retired).await.unwrap();

    let out = call_ok(
        &client,
        "remove_tags",
        json!({"id": id, "tags": ["Retired"]}),
    )
    .await;

    assert_eq!(out["tags"], json!([]));
}

#[tokio::test]
async fn server_identifies_as_folio() {
    let fx = Fixture::new().await;
    let client = fx.in_process_client().await;

    let info = client.peer_info().unwrap();

    let server = info.server_info.as_ref().unwrap();
    assert_eq!(server.name, "folio");
    assert_eq!(server.version, env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn trusted_stdio_mutations_have_no_user_actor() {
    let fx = Fixture::new().await;
    let doc = fx.insert("Invoice", "total due").await;
    let client = fx.in_process_client().await;
    common::call_ok(
        &client,
        "add_tags",
        json!({
            "id": doc.id.to_string(), "tags": ["system actor"], "actor": "forged"
        }),
    )
    .await;
    assert_eq!(*fx.tag_actors.lock().unwrap(), vec![None]);
}
