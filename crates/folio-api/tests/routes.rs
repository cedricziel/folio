//! Drives the real router to make sure path-parameter routes match.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use folio_api::{create_router, AppState, AuthState, IngestionState};
use folio_core::ports::{keys, DocumentStore};
use folio_core::{
    models::{CreateCorrespondent, CreateDocument, CreateTag},
    EventBroadcaster,
};
use folio_infra::{
    Config, Database, DynamicEmbeddingService, DynamicOcrService, ObjectStoreDocumentStore,
    PipelineContext, ThumbnailService, WebhookClient,
};
use tower::ServiceExt;
use uuid::Uuid;

struct TestApp {
    router: Router,
    state: AppState,
    vault: folio_core::models::VaultId,
    vault_repository: Arc<dyn folio_core::ports::VaultRepository>,
    dir: std::path::PathBuf,
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn test_app() -> TestApp {
    test_app_with_auth(false).await
}

async fn test_app_with_auth(auth_required: bool) -> TestApp {
    let dir = std::env::temp_dir().join(format!("folio-routes-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let storage_path = dir.to_str().unwrap().to_string();

    // A file-backed database: each pooled connection to `:memory:` would see its own empty DB.
    let database = Database::connect(&format!("sqlite://{}/test.db", storage_path))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let repos = database.repositories().await;
    let vault = repos.vaults.instance_default().await.unwrap();
    let vaults = Arc::new(folio_core::services::VaultService::new(
        repos.vaults.clone(),
        repos.members,
    ));
    let auth_store = repos.auth.clone();

    let store: Arc<dyn DocumentStore> = Arc::new(ObjectStoreDocumentStore::in_memory());
    let ai = Config::default().ai;
    let pipeline = PipelineContext::new(repos.settings.clone(), ai);
    let ingestion = IngestionState::new(
        repos.sources,
        repos.ledger,
        repos.ingest_outcomes,
        Vec::new(),
    );
    let state = AppState::new(
        repos.archival,
        repos.metadata_fields,
        repos.metadata_options,
        repos.metadata_values,
        repos.saved_views,
        repos.documents,
        repos.entries,
        repos.upload_receipts,
        repos.tags,
        repos.correspondents,
        repos.document_types,
        repos.settings.clone(),
        Arc::new(DynamicOcrService::new(pipeline.clone())),
        Arc::new(DynamicEmbeddingService::new(pipeline.clone())),
        repos.search,
        pipeline,
        Arc::new(ThumbnailService::new(store.clone())),
        repos.jobs,
        store,
        EventBroadcaster::default(),
        repos.outbox,
        repos.workflows,
        WebhookClient::new(),
        "http://folio.test".to_string(),
        ingestion,
        vaults,
        repos.access,
    );

    TestApp {
        router: create_router(
            state.clone(),
            AuthState::new(auth_store, None).with_required(auth_required),
        ),
        state,
        vault,
        vault_repository: repos.vaults.clone(),
        dir,
    }
}

async fn request(app: &TestApp, method: &str, uri: &str) -> (StatusCode, serde_json::Value) {
    request_json(app, method, uri, None).await
}

async fn request_json(
    app: &TestApp,
    method: &str,
    uri: &str,
    body: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn get_document_by_id_returns_the_document() {
    let app = test_app().await;
    let doc = app
        .state
        .document_repo
        .create(
            app.vault,
            CreateDocument {
                title: "Invoice".into(),
                content: None,
                original_filename: None,
                mime_type: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let (status, body) = request(&app, "GET", &format!("/api/documents/{}", doc.id)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["title"], "Invoice");
}

async fn tagged_document(
    app: &TestApp,
    title: &str,
    tags: &[Uuid],
) -> folio_core::models::Document {
    tagged_document_in(app, app.vault, title, tags).await
}

async fn tagged_document_in(
    app: &TestApp,
    vault: folio_core::models::VaultId,
    title: &str,
    tags: &[Uuid],
) -> folio_core::models::Document {
    let doc = app
        .state
        .document_repo
        .create(
            vault,
            CreateDocument {
                title: title.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for tag in tags {
        app.state
            .document_repo
            .add_tag(vault, doc.id, *tag)
            .await
            .unwrap();
    }
    doc
}

async fn tag_in(app: &TestApp, vault: folio_core::models::VaultId, name: &str) -> Uuid {
    let tag = CreateTag {
        name: name.into(),
        color: None,
    };
    app.state.tag_repo.create(vault, tag).await.unwrap().id
}

/// Tags `Tax` and `Home` on documents "both", "tax only" and "none"
async fn tax_and_home_documents(app: &TestApp) -> (Uuid, Uuid) {
    let (tax, home) = (
        tag_in(app, app.vault, "Tax").await,
        tag_in(app, app.vault, "Home").await,
    );
    tagged_document(app, "both", &[tax, home]).await;
    tagged_document(app, "tax only", &[tax]).await;
    tagged_document(app, "none", &[]).await;
    (tax, home)
}

#[tokio::test]
async fn documents_carry_their_tags_and_names() {
    let app = test_app().await;
    let tag = app
        .state
        .tag_repo
        .create(
            app.vault,
            CreateTag {
                name: "Taxes".into(),
                color: Some("#ff5733".into()),
            },
        )
        .await
        .unwrap();
    let sender = app
        .state
        .correspondent_repo
        .create(
            app.vault,
            CreateCorrespondent {
                name: "Finanzamt".into(),
            },
        )
        .await
        .unwrap();
    let doc = tagged_document(&app, "Bescheid", &[tag.id]).await;
    app.state
        .document_repo
        .update(
            app.vault,
            doc.id,
            folio_core::models::UpdateDocument {
                correspondent_id: Some(Some(sender.id)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let (_, list) = request(&app, "GET", "/api/documents").await;
    let (_, one) = request(&app, "GET", &format!("/api/documents/{}", doc.id)).await;

    for body in [&list["documents"][0], &one] {
        assert_eq!(body["correspondent"], "Finanzamt");
        assert_eq!(body["document_type"], serde_json::Value::Null);
        assert_eq!(body["tags"][0]["id"], tag.id.to_string());
        assert_eq!(body["tags"][0]["name"], "Taxes");
        assert_eq!(body["tags"][0]["color"], "#ff5733");
    }
}

#[tokio::test]
async fn document_ids_take_the_list_filters_and_carry_only_ids() {
    let app = test_app().await;
    let (tax, home) = tax_and_home_documents(&app).await;
    let (_, listed) = request(&app, "GET", &format!("/api/documents?tag_id={tax}")).await;
    let mut expected: Vec<String> = listed["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|document| document["id"].as_str().unwrap().to_owned())
        .collect();
    expected.sort();

    let (status, body) = request(&app, "GET", &format!("/api/documents/ids?tag_id={tax}")).await;
    assert_eq!(status, StatusCode::OK);
    let mut ids: Vec<String> = body["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect();
    ids.sort();
    assert_eq!(ids, expected);
    assert_eq!(body.as_object().unwrap().len(), 1, "only ids: {body}");

    let uri = format!("/api/documents/ids?tag_id={tax}&tag_id={home}");
    let (_, body) = request(&app, "GET", &uri).await;
    assert_eq!(body["ids"].as_array().unwrap().len(), 1);

    let (status, _) = request(&app, "GET", "/api/documents/ids?tag_id=not-a-uuid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn list_filters_by_all_tags_and_by_creation_time() {
    let app = test_app().await;
    let (tax, home) = tax_and_home_documents(&app).await;

    let (_, body) = request(&app, "GET", &format!("/api/documents?tag_id={tax}")).await;
    assert_eq!(sorted_titles(&body), ["both", "tax only"]);

    let uri = format!("/api/documents?tag_id={tax}&tag_id={home}");
    let (_, body) = request(&app, "GET", &uri).await;
    assert_eq!(sorted_titles(&body), ["both"]);
    assert_eq!(body["count"], 1);

    let (_, body) = request(
        &app,
        "GET",
        "/api/documents?created_after=2999-01-01T00:00:00Z",
    )
    .await;
    assert_eq!(body["count"], 0);
    let (_, body) = request(
        &app,
        "GET",
        "/api/documents?created_before=2999-01-01T00:00:00Z",
    )
    .await;
    assert_eq!(body["count"], 3);
}

fn sorted_titles(body: &serde_json::Value) -> Vec<String> {
    let mut titles: Vec<String> = body["documents"]
        .as_array()
        .unwrap_or_else(|| panic!("no documents in {body}"))
        .iter()
        .map(|d| d["title"].as_str().unwrap().to_string())
        .collect();
    titles.sort();
    titles
}

#[tokio::test]
async fn list_filters_survive_unknown_parameters_in_any_order() {
    let app = test_app().await;
    let (tax, home) = tax_and_home_documents(&app).await;

    for (query, expected) in [
        (format!("tag_id={tax}&limit=20"), &["both", "tax only"][..]),
        (format!("limit=20&tag_id={tax}"), &["both", "tax only"]),
        (format!("tag_id={tax}&limit=1&tag_id={home}"), &["both"]),
        (format!("tag_id={home}&tag_id={tax}&limit=20"), &["both"]),
        (format!("limit=20&tag_id={home}&tag_id={tax}"), &["both"]),
        ("limit=20".to_string(), &["both", "none", "tax only"]),
    ] {
        let (status, body) = request(&app, "GET", &format!("/api/documents?{query}")).await;
        assert_eq!(status, StatusCode::OK, "{query}");
        assert_eq!(sorted_titles(&body), expected, "{query}");
    }
}

#[tokio::test]
async fn list_rejects_invalid_filters_next_to_unknown_parameters() {
    let app = test_app().await;
    let tag = Uuid::new_v4();
    for query in [
        "limit=20&tag_id=nope".to_string(),
        "tag_id=nope&limit=20".to_string(),
        format!("tag_id={tag}&created_after=yesterday&limit=20"),
        format!("limit=20&created_after=yesterday&tag_id={tag}"),
        format!("tag_id={tag}&tag_id=nope"),
    ] {
        let (status, body) = request(&app, "GET", &format!("/api/documents?{query}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
        assert!(body["error"].as_str().is_some(), "{query}");
    }
}

#[tokio::test]
async fn list_filters_stay_inside_the_requested_vault() {
    let app = test_app().await;
    let mine = tag_in(&app, app.vault, "Tax").await;
    tagged_document(&app, "mine", &[mine]).await;
    let other = app.vault_repository.create("Other", None).await.unwrap().id;
    let theirs = tag_in(&app, other, "Tax").await;
    tagged_document_in(&app, other, "theirs", &[theirs]).await;

    for (query, expected) in [
        (format!("tag_id={mine}&limit=20"), &["mine"][..]),
        (format!("limit=20&tag_id={theirs}"), &[]),
        (format!("tag_id={mine}&tag_id={theirs}"), &[]),
        ("limit=20".to_string(), &["mine"]),
    ] {
        let (status, body) = request(&app, "GET", &format!("/api/documents?{query}")).await;
        assert_eq!(status, StatusCode::OK, "{query}");
        assert_eq!(sorted_titles(&body), expected, "{query}");
    }
}

#[tokio::test]
async fn list_rejects_malformed_filters() {
    let app = test_app().await;
    for query in [
        "tag_id=nope",
        "created_after=yesterday",
        "correspondent_id=1",
    ] {
        let (status, body) = request(&app, "GET", &format!("/api/documents?{query}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
        assert!(body["error"].as_str().is_some());
    }
}

#[tokio::test]
async fn unknown_ids_reach_the_handlers_and_return_json_404() {
    let app = test_app().await;
    let id = Uuid::new_v4();

    for path in [
        format!("/api/documents/{id}"),
        format!("/api/tags/{id}"),
        format!("/api/correspondents/{id}"),
        format!("/api/document_types/{id}"),
    ] {
        let (status, body) = request(&app, "GET", &path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(
            body["error"].as_str().is_some(),
            "{path} returned the router fallback instead of a handler error"
        );
    }
}

#[tokio::test]
async fn nested_tag_route_matches() {
    let app = test_app().await;
    let tag = app
        .state
        .tag_repo
        .create(
            app.vault,
            CreateTag {
                name: "Nested".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let doc = tagged_document(&app, "Tagged", &[tag.id]).await;
    let (doc_id, tag_id) = (doc.id, tag.id);

    let (status, _) = request(
        &app,
        "POST",
        &format!("/api/documents/{doc_id}/tags/{tag_id}"),
    )
    .await;

    assert_eq!(status, StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn extractor_rejections_return_the_json_error_body() {
    let app = test_app().await;

    for (method, uri, body, expected) in [
        (
            "POST",
            "/api/tags",
            Some("{}"),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("POST", "/api/tags", Some("{"), StatusCode::BAD_REQUEST),
        ("GET", "/api/tags/not-a-uuid", None, StatusCode::BAD_REQUEST),
        (
            "GET",
            "/api/documents?correspondent_id=nope",
            None,
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, json) = request_json(&app, method, uri, body).await;
        assert_eq!(status, expected, "{method} {uri}");
        assert!(
            json["error"].as_str().is_some_and(|e| !e.is_empty()),
            "{method} {uri} did not return an ErrorResponse: {json}"
        );
    }
}

#[tokio::test]
async fn deletes_return_no_content_with_an_empty_body() {
    let app = test_app().await;
    let tag = app
        .state
        .tag_repo
        .create(
            app.vault,
            CreateTag {
                name: "Tax".into(),
                color: None,
            },
        )
        .await
        .unwrap();

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/tags/{}", tag.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.is_empty());
}

#[tokio::test]
async fn update_tag_clears_color_on_null_and_keeps_it_when_absent() {
    let app = test_app().await;
    let tag = app
        .state
        .tag_repo
        .create(
            app.vault,
            CreateTag {
                name: "Tax".into(),
                color: Some("#ff5733".into()),
            },
        )
        .await
        .unwrap();
    let uri = format!("/api/tags/{}", tag.id);

    let (status, body) = request_json(&app, "PUT", &uri, Some(r#"{"name":"Taxes"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["color"], "#ff5733");

    let (status, body) = request_json(&app, "PUT", &uri, Some(r#"{"color":null}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["color"].is_null());
    assert_eq!(body["name"], "Taxes");
}

async fn stored_document(app: &TestApp, filename: &str, bytes: &'static [u8]) -> Uuid {
    let doc = app
        .state
        .document_repo
        .create(
            app.vault,
            CreateDocument {
                title: "Stored".into(),
                content: None,
                original_filename: Some(filename.into()),
                mime_type: Some("application/pdf".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let key = keys::original(doc.id);
    app.state.store.put(&key, bytes.into()).await.unwrap();
    app.state
        .document_repo
        .set_file_path(app.vault, doc.id, &key)
        .await
        .unwrap();
    doc.id
}

#[tokio::test]
async fn download_streams_the_stored_file_with_headers() {
    let app = test_app().await;
    let id = stored_document(&app, "Größe.pdf", b"%PDF-1.4 body").await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/documents/{id}/download"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(headers["content-type"], "application/pdf");
    assert_eq!(headers["content-length"], "13");
    let disposition = headers["content-disposition"].to_str().unwrap();
    assert!(
        disposition.ends_with("filename*=UTF-8''Gr%C3%B6%C3%9Fe.pdf"),
        "{disposition}"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], b"%PDF-1.4 body");
}

#[tokio::test]
async fn thumbnail_route_streams_jpeg_from_the_store() {
    let app = test_app().await;
    let id = stored_document(&app, "a.pdf", b"x").await;
    app.state
        .store
        .put(&keys::thumbnail(id), b"jpeg-bytes".as_slice().into())
        .await
        .unwrap();

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/documents/{id}/thumbnail"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/jpeg");
    assert_eq!(response.headers()["content-length"], "10");
}

#[tokio::test]
async fn page_thumbnail_route_streams_jpeg_from_the_store() {
    let app = test_app().await;
    let id = stored_document(&app, "a.pdf", b"x").await;
    app.state
        .store
        .put(&keys::page(id, 2), b"page-jpeg".as_slice().into())
        .await
        .unwrap();

    let response = raw_get(&app, &format!("/documents/{id}/page/2/thumbnail")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/jpeg");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], b"page-jpeg");
}

#[tokio::test]
async fn thumbnail_route_ignores_objects_outside_the_document_prefix() {
    let app = test_app().await;
    let id = stored_document(&app, "a.pdf", b"x").await;
    app.state
        .store
        .put(
            &format!("thumbnails/{id}.jpg"),
            b"old-jpeg".as_slice().into(),
        )
        .await
        .unwrap();

    let response = raw_get(&app, &format!("/documents/{id}/thumbnail")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/svg+xml");
}

#[tokio::test]
async fn page_thumbnail_route_ignores_objects_outside_the_document_prefix() {
    let app = test_app().await;
    let id = stored_document(&app, "a.pdf", b"x").await;
    app.state
        .store
        .put(
            &format!("thumbnails/{id}-p2.jpg"),
            b"old-jpeg".as_slice().into(),
        )
        .await
        .unwrap();

    let response = raw_get(&app, &format!("/documents/{id}/page/2/thumbnail")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/svg+xml");
}

#[tokio::test]
async fn deleting_a_document_retains_original_and_previews_in_trash() {
    let app = test_app().await;
    let id = stored_document(&app, "a.pdf", b"original").await;
    let other = stored_document(&app, "b.pdf", b"keep me").await;
    app.state
        .document_repo
        .set_page_count(app.vault, id, 3)
        .await
        .unwrap();
    for key in [keys::thumbnail(id), keys::page(id, 2), keys::page(id, 3)] {
        app.state
            .store
            .put(&key, b"t".as_slice().into())
            .await
            .unwrap();
    }

    let (status, _) = request(&app, "DELETE", &format!("/api/documents/{id}")).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.state
            .store
            .list(&keys::document_prefix(id))
            .await
            .unwrap()
            .len(),
        4
    );
    assert!(app
        .state
        .document_repo
        .find_by_id(app.vault, id)
        .await
        .unwrap()
        .is_none());
    assert!(app
        .state
        .document_repo
        .find_trashed_by_id(app.vault, id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        raw_get(&app, &format!("/documents/{id}/thumbnail"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(app
        .state
        .store
        .exists(&keys::original(other))
        .await
        .unwrap());
}

#[tokio::test]
async fn full_router_enforces_auth_when_required() {
    let app = test_app_with_auth(true).await;
    let (status, _) = request(&app, "GET", "/api/tags").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = request(&app, "GET", "/api/sources").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "sources API sits behind auth"
    );
    let (status, _) = request(&app, "GET", "/settings/sources").await;
    assert_ne!(status, StatusCode::OK, "sources page sits behind auth");
    let (status, _) = request(&app, "GET", "/.well-known/oauth-authorization-server").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = request(&app, "GET", "/api/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    for probe in ["/healthz", "/readyz"] {
        let (status, _) = request(&app, "GET", probe).await;
        assert_eq!(status, StatusCode::OK, "{probe} must stay unauthenticated");
    }
}

async fn document_with_file(app: &TestApp, mime: &str, name: &str, data: &[u8]) -> Uuid {
    let doc = app
        .state
        .document_repo
        .create(
            app.vault,
            CreateDocument {
                title: "File".into(),
                content: None,
                original_filename: Some(name.into()),
                mime_type: Some(mime.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let path = keys::original(doc.id);
    app.state
        .store
        .put(&path, axum::body::Bytes::copy_from_slice(data))
        .await
        .unwrap();
    app.state
        .document_repo
        .set_file_path(app.vault, doc.id, &path)
        .await
        .unwrap();
    doc.id
}

async fn raw_get(app: &TestApp, uri: &str) -> axum::response::Response {
    app.router
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn file_responses_are_hardened_against_active_content() {
    let app = test_app().await;
    let html = document_with_file(&app, "text/html", "x.html", b"<script>1</script>").await;
    let svg = document_with_file(&app, "image/svg+xml", "x.svg", b"<svg onload=1/>").await;
    let png = document_with_file(&app, "image/png", "x.png", b"png").await;

    for uri in [
        format!("/api/documents/{html}/download"),
        format!("/documents/{html}/thumbnail"),
        format!("/documents/{svg}/thumbnail"),
        format!("/documents/{png}/thumbnail"),
    ] {
        let res = raw_get(&app, &uri).await;
        let h = res.headers();
        assert_eq!(h["x-content-type-options"], "nosniff", "{uri}");
        assert_eq!(
            h["content-security-policy"], "default-src 'none'; sandbox",
            "{uri}"
        );
    }

    // Untrusted types never render inline.
    for (uri, kind) in [
        (format!("/documents/{svg}/thumbnail"), "image/svg+xml"),
        (format!("/api/documents/{html}/download"), "text/html"),
    ] {
        let res = raw_get(&app, &uri).await;
        let disposition = res.headers()["content-disposition"].to_str().unwrap();
        assert!(
            disposition.starts_with("attachment"),
            "{kind}: {disposition}"
        );
    }
    // Allowlisted image types stay inline so <img> thumbnails keep working.
    let res = raw_get(&app, &format!("/documents/{png}/thumbnail")).await;
    assert_eq!(res.headers()["content-type"], "image/png");
    assert!(res.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .starts_with("inline"));
}

#[tokio::test]
async fn classification_forms_reject_invalid_or_missing_ids_without_clearing() {
    let app = test_app().await;
    let correspondent = app
        .state
        .correspondent_repo
        .create(
            app.vault,
            CreateCorrespondent {
                name: "Existing sender".into(),
            },
        )
        .await
        .unwrap();
    let document_type = app
        .state
        .document_type_repo
        .create(
            app.vault,
            folio_core::models::CreateDocumentType {
                name: "Existing type".into(),
            },
        )
        .await
        .unwrap();
    let document = app
        .state
        .document_repo
        .create(
            app.vault,
            CreateDocument {
                title: "Classified document".into(),
                correspondent_id: Some(correspondent.id),
                document_type_id: Some(document_type.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    for prefix in ["/documents", "/api/documents"] {
        for (route, field, id) in [
            ("correspondent", "correspondent_id", correspondent.id),
            ("document_type", "document_type_id", document_type.id),
        ] {
            let uri = format!("{prefix}/{}/{route}", document.id);
            for body in [format!("{field}=not-a-uuid"), String::new()] {
                let response = app
                    .router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri(&uri)
                            .header("content-type", "application/x-www-form-urlencoded")
                            .body(Body::from(body))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
                let saved = app
                    .state
                    .document_repo
                    .find_by_id(app.vault, document.id)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(saved.correspondent_id, Some(correspondent.id));
                assert_eq!(saved.document_type_id, Some(document_type.id));
            }
            for value in [String::new(), id.to_string()] {
                let response = app
                    .router
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri(&uri)
                            .header("content-type", "application/x-www-form-urlencoded")
                            .body(Body::from(format!("{field}={value}")))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::SEE_OTHER, "{uri}");
                let saved = app
                    .state
                    .document_repo
                    .find_by_id(app.vault, document.id)
                    .await
                    .unwrap()
                    .unwrap();
                let expected = if value.is_empty() { None } else { Some(id) };
                if field == "correspondent_id" {
                    assert_eq!(saved.correspondent_id, expected);
                    assert_eq!(saved.document_type_id, Some(document_type.id));
                } else {
                    assert_eq!(saved.document_type_id, expected);
                    assert_eq!(saved.correspondent_id, Some(correspondent.id));
                }
            }
        }
    }
}

#[tokio::test]
async fn taxonomy_trash_retains_document_classifications_and_restore_conflicts() {
    let app = test_app().await;
    let (status, tag) =
        request_json(&app, "POST", "/api/tags", Some(r#"{"name":"Retained"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tag["created_at"], tag["updated_at"]);
    assert!(tag["vault_id"].is_string());
    let id = tag["id"].as_str().unwrap();
    let document = tagged_document(&app, "Kept document", &[Uuid::parse_str(id).unwrap()]).await;
    assert_eq!(
        request(&app, "DELETE", &format!("/api/tags/{id}")).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "GET", &format!("/api/tags/{id}")).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, trash) = request(&app, "GET", "/api/tags/trash").await;
    assert_eq!(trash[0]["id"], id);
    assert!(trash[0]["deleted_at"].is_string());
    let (_, retained) = request(&app, "GET", &format!("/api/documents/{}", document.id)).await;
    assert_eq!(retained["tags"][0]["name"], "Retained");
    assert!(retained["tags"][0]["deleted_at"].is_string());
    assert_eq!(
        request_json(&app, "POST", "/api/tags", Some(r#"{"name":"Retained"}"#))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "POST", &format!("/api/tags/{id}/restore"))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let (_, trash) = request(&app, "GET", "/api/tags/trash").await;
    assert_eq!(trash[0]["id"], id);
}

async fn classification_form_emits_a_transition(prefix: &str) {
    let app = test_app().await;
    let correspondent = app
        .state
        .correspondent_repo
        .create(
            app.vault,
            CreateCorrespondent {
                name: "Event sender".into(),
            },
        )
        .await
        .unwrap();
    let document = app
        .state
        .document_repo
        .create(
            app.vault,
            CreateDocument {
                title: "HTTP transition".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("{prefix}/{}/correspondent", document.id))
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(format!("correspondent_id={}", correspondent.id)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let events = app
        .state
        .outbox
        .replay(0, chrono::Utc::now() - chrono::Duration::days(1), 100)
        .await
        .unwrap();
    let transition = events
        .iter()
        .filter_map(|event| event.envelope.as_ref())
        .find(|event| event.event_type() == "document.correspondent_changed")
        .unwrap();
    assert_eq!(transition.vault_id(), app.vault);
    assert_eq!(transition.document_id(), Some(document.id));
    assert!(transition.data()["before_id"].is_null());
    assert_eq!(transition.data()["after_id"], correspondent.id.to_string());
}

#[tokio::test]
async fn html_classification_form_publishes_the_committed_transition() {
    classification_form_emits_a_transition("/documents").await;
}

#[tokio::test]
async fn api_classification_form_publishes_the_committed_transition() {
    classification_form_emits_a_transition("/api/documents").await;
}

#[tokio::test]
async fn document_detail_distinguishes_empty_ocr_from_indexing_failure() {
    use folio_core::models::{ProcessingStage, ProcessingStatus};
    let app = test_app().await;
    let doc = app
        .state
        .document_repo
        .create(
            app.vault,
            CreateDocument {
                title: "Processing detail".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    app.state
        .document_repo
        .complete_ocr(app.vault, doc.id, "")
        .await
        .unwrap();
    app.state
        .document_repo
        .set_stage_status(
            app.vault,
            doc.id,
            ProcessingStage::Embed,
            ProcessingStatus::Failed,
        )
        .await
        .unwrap();
    let response = raw_get(&app, &format!("/documents/{}", doc.id)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(html.contains("Text extraction"));
    assert!(html.contains("Completed with no text"));
    assert!(html.contains("Search indexing"));
    assert!(html.contains("Last attempt failed"));
    assert!(html.contains("The original file is unchanged."));
}

#[tokio::test]
async fn original_exports_keep_both_revisions_after_the_current_pointer_changes() {
    let h = test_app().await;
    let doc = h
        .state
        .document_repo
        .create_for_upload(
            h.vault,
            CreateDocument {
                title: "Updated invoice".into(),
                original_filename: Some("first.pdf".into()),
                mime_type: Some("application/pdf".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let first = b"unchanged bytes from revision one";
    let next = b"new bytes from revision two";
    let next_id = Uuid::new_v4();
    let first_key = keys::original(doc.id);
    let next_key = keys::revision_original(doc.id, next_id);
    let first_identity =
        folio_core::models::FileIdentity::from_bytes(&first_key, "application/pdf", first);
    let next_identity =
        folio_core::models::FileIdentity::from_bytes(&next_key, "application/pdf", next);

    h.state
        .store
        .put(&first_key, first.as_slice().into())
        .await
        .unwrap();
    h.state
        .store
        .put(&next_key, next.as_slice().into())
        .await
        .unwrap();
    h.state
        .document_repo
        .finalize_verified_upload(h.vault, doc.id, &first_identity)
        .await
        .unwrap();

    let Database::Sqlite(pool) =
        Database::connect(&format!("sqlite://{}/test.db", h.dir.display()))
            .await
            .unwrap()
    else {
        panic!("expected the file-backed route fixture");
    };
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query(
        "INSERT INTO document_files (id, document_id, vault_id, role, storage_key, original_filename,
             mime_type, verification_state, sha256, byte_size)
         VALUES (?, ?, ?, 'original', ?, 'second.pdf', 'application/pdf', 'verified', ?, ?)",
    )
    .bind(next_id.to_string())
    .bind(doc.id.to_string())
    .bind(h.vault.to_string())
    .bind(&next_identity.storage_key)
    .bind(&next_identity.sha256)
    .bind(i64::try_from(next_identity.byte_size).unwrap())
    .execute(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO document_revisions (id, document_id, vault_id, number, original_file_id, created_at)
         VALUES (?, ?, ?, 2, ?, '2026-11-22T21:00:00Z')",
    )
    .bind(next_id.to_string())
    .bind(doc.id.to_string())
    .bind(h.vault.to_string())
    .bind(next_id.to_string())
    .execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE documents SET current_revision_id = ?, file_path = ?, original_filename = 'second.pdf' WHERE id = ?")
        .bind(next_id.to_string()).bind(&next_identity.storage_key).bind(doc.id.to_string())
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();

    let root = format!("/api/documents/{}", doc.id);
    let history = format!("{root}/revisions");
    let (status, response) = request(&h, "GET", &history).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["revisions"][0]["id"], next_id.to_string());
    assert_eq!(response["revisions"][0]["is_current"], true);
    assert_eq!(response["revisions"][1]["id"], doc.id.to_string());
    assert_eq!(response["revisions"][1]["is_current"], false);
    for (uri, contents) in [
        (format!("{root}/download"), next.as_slice()),
        (format!("{history}/{next_id}/download"), next.as_slice()),
        (format!("{history}/{}/download", doc.id), first.as_slice()),
    ] {
        let response = h
            .router
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), contents);
    }
}

#[tokio::test]
async fn replacement_routes_share_the_receipt_and_preserve_the_original() {
    let h = test_app().await;
    let document = h
        .state
        .document_repo
        .create_for_upload(h.vault, Default::default())
        .await
        .unwrap();
    let original_key = keys::original(document.id);
    let original = b"first original";
    h.state
        .store
        .put(&original_key, original.as_slice().into())
        .await
        .unwrap();
    let identity =
        folio_core::models::FileIdentity::from_bytes(&original_key, "application/pdf", original);
    h.state
        .document_repo
        .finalize_verified_upload(h.vault, document.id, &identity)
        .await
        .unwrap();

    let default_path = format!("/api/documents/{}/revisions", document.id);
    let scoped_path = format!(
        "/api/vaults/{}/documents/{}/revisions",
        h.vault, document.id
    );
    let send = |path: &str, expected: Uuid, key: Option<Uuid>| {
        let multipart = format!(
            "--revision-boundary\r\nContent-Disposition: form-data; name=\"expected_current_revision_id\"\r\n\r\n{expected}\r\n\
             --revision-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"replacement.pdf\"\r\n\
             Content-Type: application/pdf\r\n\r\nreplacement bytes\r\n--revision-boundary--\r\n",
        );
        let mut request = Request::post(path).header(
            "content-type",
            "multipart/form-data; boundary=revision-boundary",
        );
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key.to_string());
        }
        h.router
            .clone()
            .oneshot(request.body(Body::from(multipart)).unwrap())
    };
    let key = Uuid::new_v4();
    let inaccessible_path = format!(
        "/api/vaults/{}/documents/{}/revisions",
        Uuid::new_v4(),
        document.id
    );
    assert_eq!(
        send(&inaccessible_path, document.id, Some(key))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );

    assert_eq!(
        send(&default_path, document.id, None)
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        send(&default_path, Uuid::nil(), Some(key))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let created = send(&default_path, document.id, Some(key)).await.unwrap();
    let replayed = send(&scoped_path, document.id, Some(key)).await.unwrap();
    let enabled = folio_infra::queue::REVISION_DISPATCH_ENABLED;
    if enabled {
        assert_eq!(created.status(), StatusCode::CREATED);
        assert_eq!(replayed.status(), StatusCode::OK);
        let created = axum::body::to_bytes(created.into_body(), 4096)
            .await
            .unwrap();
        let replayed = axum::body::to_bytes(replayed.into_body(), 4096)
            .await
            .unwrap();
        assert_eq!(created, replayed);
        assert_eq!(
            send(&scoped_path, document.id, Some(Uuid::new_v4()))
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT,
        );
    } else {
        assert_eq!(created.status(), StatusCode::CONFLICT);
        assert_eq!(replayed.status(), StatusCode::CONFLICT);
    }

    let revisions = h
        .state
        .document_repo
        .list_revisions(h.vault, document.id, None, 10)
        .await
        .unwrap();
    assert_eq!(revisions.len(), if enabled { 2 } else { 1 });
    assert_eq!(revisions.last().unwrap().id, document.id);
    assert!(revisions[0].is_current);
    if enabled {
        assert_ne!(revisions[0].id, document.id);
        assert_eq!(revisions[0].number, 2);
        assert_eq!(
            h.state
                .store
                .get_bytes(&revisions[0].original.storage_key)
                .await
                .unwrap(),
            b"replacement bytes",
        );
    }
    assert_eq!(
        h.state.store.get_bytes(&original_key).await.unwrap(),
        original
    );
    let staged = h
        .state
        .store
        .list(&keys::document_prefix(document.id))
        .await
        .unwrap();
    assert_eq!(staged.len(), revisions.len());
    assert!(revisions
        .iter()
        .all(|revision| staged.contains(&revision.original.storage_key)));
}

#[tokio::test]
async fn revision_history_lists_metadata_and_downloads_the_original() {
    let h = test_app().await;
    let doc = h
        .state
        .document_repo
        .create_for_upload(
            h.vault,
            CreateDocument {
                title: "Invoice".into(),
                original_filename: Some("invoice.pdf".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let key = keys::original(doc.id);
    let original = b"the bytes of revision one";
    h.state
        .store
        .put(&key, original.as_slice().into())
        .await
        .unwrap();
    let identity = folio_core::models::FileIdentity::from_bytes(&key, "application/pdf", original);
    h.state
        .document_repo
        .finalize_verified_upload(h.vault, doc.id, &identity)
        .await
        .unwrap();

    let history = format!("/api/documents/{}/revisions", doc.id);
    let response = h
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(&history)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 16)
        .await
        .unwrap();
    let page: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let revision = &page["revisions"][0];
    assert_eq!(revision["number"], 1);
    assert_eq!(revision["is_current"], true);
    assert_eq!(revision["original"]["filename"], "invoice.pdf");
    assert_eq!(revision["original"]["byte_size"], original.len());
    assert_eq!(revision["original"]["verification"], "verified");
    assert!(revision["original"].get("storage_key").is_none());

    let revision_id = revision["id"].as_str().unwrap();
    let download = format!("{history}/{revision_id}/download");
    let response = h
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(&download)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/pdf");
    assert!(response.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .contains("invoice.pdf"));
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .as_ref(),
        original
    );

    let scoped_history = format!("/api/vaults/{}/documents/{}/revisions", h.vault, doc.id);
    let scoped_download = format!("{scoped_history}/{revision_id}/download");
    let response = h
        .router
        .clone()
        .oneshot(Request::get(scoped_download).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .as_ref(),
        original,
    );

    let wrong_scope = format!(
        "/api/vaults/{}/documents/{}/revisions",
        Uuid::new_v4(),
        doc.id
    );
    let response = h
        .router
        .clone()
        .oneshot(Request::get(wrong_scope).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    h.state.document_repo.delete(h.vault, doc.id).await.unwrap();
    let response = h
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(&download)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
