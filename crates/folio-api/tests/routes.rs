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
    let ingestion = IngestionState::new(repos.sources, repos.ledger, Vec::new());
    let state = AppState::new(
        repos.documents,
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
    let doc = app
        .state
        .document_repo
        .create(
            app.vault,
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
            .add_tag(app.vault, doc.id, *tag)
            .await
            .unwrap();
    }
    doc
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
async fn list_filters_by_all_tags_and_by_creation_time() {
    let app = test_app().await;
    let make = |name: &str| {
        app.state.tag_repo.create(
            app.vault,
            CreateTag {
                name: name.into(),
                color: None,
            },
        )
    };
    let (tax, home) = (make("Tax").await.unwrap(), make("Home").await.unwrap());
    tagged_document(&app, "both", &[tax.id, home.id]).await;
    tagged_document(&app, "tax only", &[tax.id]).await;
    tagged_document(&app, "none", &[]).await;

    let titles = |body: &serde_json::Value| {
        let mut titles: Vec<String> = body["documents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["title"].as_str().unwrap().to_string())
            .collect();
        titles.sort();
        titles
    };

    let (_, body) = request(&app, "GET", &format!("/api/documents?tag_id={}", tax.id)).await;
    assert_eq!(titles(&body), ["both", "tax only"]);

    let uri = format!("/api/documents?tag_id={}&tag_id={}", tax.id, home.id);
    let (_, body) = request(&app, "GET", &uri).await;
    assert_eq!(titles(&body), ["both"]);
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
