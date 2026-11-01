//! Drives the real router to make sure path-parameter routes match.

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use folio_api::{create_router, AppState};
use folio_core::{
    models::{CreateDocument, CreateTag},
    EventBroadcaster,
};
use folio_infra::{
    create_pool, run_migrations, Config, DynamicEmbeddingService, DynamicOcrService, JobQueue,
    SqliteCorrespondentRepository, SqliteDocumentRepository, SqliteDocumentTypeRepository,
    SqliteSearchService, SqliteSettingsRepository, SqliteTagRepository, ThumbnailService,
};
use tower::ServiceExt;
use uuid::Uuid;

struct TestApp {
    router: Router,
    state: AppState,
    dir: std::path::PathBuf,
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn test_app() -> TestApp {
    let dir = std::env::temp_dir().join(format!("folio-routes-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let storage_path = dir.to_str().unwrap().to_string();

    // A file-backed database: each pooled connection to `:memory:` would see its own empty DB.
    let pool = create_pool(&format!("sqlite://{}/test.db", storage_path))
        .await
        .unwrap();
    run_migrations(&pool).await.unwrap();

    let ai = Config::default().ai;
    let settings_repo = Arc::new(SqliteSettingsRepository::new(pool.clone()));
    let state = AppState::new(
        Arc::new(SqliteDocumentRepository::new(pool.clone())),
        Arc::new(SqliteTagRepository::new(pool.clone())),
        Arc::new(SqliteCorrespondentRepository::new(pool.clone())),
        Arc::new(SqliteDocumentTypeRepository::new(pool.clone())),
        settings_repo.clone(),
        Arc::new(DynamicOcrService::new(
            settings_repo.clone(),
            ai.clone(),
            &storage_path,
        )),
        Arc::new(DynamicEmbeddingService::new(settings_repo, ai)),
        Arc::new(SqliteSearchService::new(pool.clone())),
        Arc::new(ThumbnailService::new(&storage_path)),
        JobQueue::new(pool),
        storage_path,
        EventBroadcaster::default(),
    );

    TestApp {
        router: create_router(state.clone()),
        state,
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
        .create(CreateDocument {
            title: "Invoice".into(),
            content: None,
            original_filename: None,
            mime_type: None,
            correspondent_id: None,
            document_type_id: None,
        })
        .await
        .unwrap();

    let (status, body) = request(&app, "GET", &format!("/api/documents/{}", doc.id)).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["title"], "Invoice");
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
    let (doc_id, tag_id) = (Uuid::new_v4(), Uuid::new_v4());

    let (status, _) = request(
        &app,
        "POST",
        &format!("/api/documents/{doc_id}/tags/{tag_id}"),
    )
    .await;

    assert_eq!(status, StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn update_tag_clears_color_on_null_and_keeps_it_when_absent() {
    let app = test_app().await;
    let tag = app
        .state
        .tag_repo
        .create(CreateTag {
            name: "Tax".into(),
            color: Some("#ff5733".into()),
        })
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
