//! Shared setup for route tests: an app on a throwaway SQLite database.

#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request};
use axum::Router;
use http_body_util::BodyExt;
use uuid::Uuid;

use folio_api::{create_router, AppState, AuthState, EventBroadcaster, IngestionState};
use folio_core::models::IngestionSourceDef;
use folio_core::ports::DocumentStore;
use folio_infra::{
    Database, DynamicEmbeddingService, DynamicOcrService, ObjectStoreDocumentStore,
    PipelineContext, ThumbnailService, WebhookClient,
};

pub struct Harness {
    pub app: Router,
    pub state: AppState,
}

pub async fn harness() -> Harness {
    harness_with_sources(Vec::new()).await
}

/// A harness that requires sign-in; the first account is created through
/// `/setup?token=`[`SETUP_TOKEN`]
pub async fn harness_requiring_login() -> Harness {
    build_harness(Vec::new(), true).await
}

pub const SETUP_TOKEN: &str = "test-setup-token";

/// A harness whose configuration defines `configured` ingestion sources
pub async fn harness_with_sources(configured: Vec<IngestionSourceDef>) -> Harness {
    build_harness(configured, false).await
}

async fn build_harness(configured: Vec<IngestionSourceDef>, require_login: bool) -> Harness {
    let dir = std::env::temp_dir().join(format!("folio-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let storage = dir.to_string_lossy().to_string();

    let database = Database::connect(&format!("sqlite:{}/folio.db", storage))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let repos = database.repositories().await;
    let auth = AuthState::new(repos.auth.clone(), None)
        .with_setup_token(SETUP_TOKEN)
        .with_required(require_login);

    let ai = folio_infra::config::AiConfig {
        provider: "local".into(),
        openai_api_key: None,
        anthropic_api_key: None,
    };
    let store: Arc<dyn DocumentStore> = Arc::new(ObjectStoreDocumentStore::in_memory());
    let thumbnails = Arc::new(ThumbnailService::new(store.clone()));

    let pipeline = PipelineContext::new(repos.settings.clone(), ai);
    let ingestion = IngestionState::new(repos.sources, repos.ledger, configured);
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
        thumbnails,
        repos.jobs,
        store,
        EventBroadcaster::new(16),
        repos.workflows,
        WebhookClient::new(),
        "http://folio.test".to_string(),
        ingestion,
    );
    Harness {
        app: create_router(state.clone(), auth),
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
