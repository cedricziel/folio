//! Shared setup for route tests: an app on a throwaway SQLite database.

#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request};
use axum::Router;
use http_body_util::BodyExt;
use uuid::Uuid;

use folio_api::{create_router, AppState, EventBroadcaster};
use folio_infra::{
    create_pool, run_migrations, DynamicEmbeddingService, DynamicOcrService, JobQueue,
    SqliteCorrespondentRepository, SqliteDocumentRepository, SqliteDocumentTypeRepository,
    SqliteSearchService, SqliteSettingsRepository, SqliteTagRepository, SqliteWorkflowRepository,
    ThumbnailService, WebhookClient,
};

pub struct Harness {
    pub app: Router,
    pub state: AppState,
}

pub async fn harness() -> Harness {
    let dir = std::env::temp_dir().join(format!("folio-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let storage = dir.to_string_lossy().to_string();

    let pool = create_pool(&format!("sqlite:{}/folio.db", storage))
        .await
        .unwrap();
    run_migrations(&pool).await.unwrap();

    let settings_repo = Arc::new(SqliteSettingsRepository::new(pool.clone()));
    let ai = folio_infra::config::AiConfig {
        provider: "local".into(),
        openai_api_key: None,
        anthropic_api_key: None,
    };
    let thumbnails = Arc::new(ThumbnailService::new(&storage));
    thumbnails.init().await.unwrap();

    let state = AppState::new(
        Arc::new(SqliteDocumentRepository::new(pool.clone())),
        Arc::new(SqliteTagRepository::new(pool.clone())),
        Arc::new(SqliteCorrespondentRepository::new(pool.clone())),
        Arc::new(SqliteDocumentTypeRepository::new(pool.clone())),
        settings_repo.clone(),
        Arc::new(DynamicOcrService::new(
            settings_repo.clone(),
            ai.clone(),
            &storage,
        )),
        Arc::new(DynamicEmbeddingService::new(settings_repo, ai)),
        Arc::new(SqliteSearchService::new(pool.clone())),
        thumbnails,
        JobQueue::new(pool.clone()),
        storage,
        EventBroadcaster::new(16),
        Arc::new(SqliteWorkflowRepository::new(pool)),
        WebhookClient::new(),
        "http://folio.test".to_string(),
    );
    Harness {
        app: create_router(state.clone()),
        state,
    }
}

pub const BOUNDARY: &str = "folioboundary";

pub fn multipart(parts: &[(&str, Option<&str>, &str)]) -> Request<Body> {
    let mut body = String::new();
    for (name, filename, content) in parts {
        body.push_str(&format!("--{BOUNDARY}\r\n"));
        match filename {
            Some(f) => body.push_str(&format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"\r\nContent-Type: image/png\r\n\r\n"
            )),
            None => body.push_str(&format!(
                "Content-Disposition: form-data; name=\"{name}\"\r\n\r\n"
            )),
        }
        body.push_str(content);
        body.push_str("\r\n");
    }
    body.push_str(&format!("--{BOUNDARY}--\r\n"));
    Request::builder()
        .method("POST")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .unwrap()
}

pub async fn text(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

pub fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}
