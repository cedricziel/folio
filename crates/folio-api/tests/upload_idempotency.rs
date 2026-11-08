mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{harness, harness_storing_in, multipart, text, Harness};
use folio_infra::Database;
use tower::ServiceExt;
use uuid::Uuid;

fn upload(uri: &str, key: Option<&str>, content: &str) -> Request<Body> {
    let mut request = multipart(&[("title", None, "Scan"), ("file", Some("scan.png"), content)]);
    *request.uri_mut() = uri.parse().unwrap();
    if let Some(key) = key {
        request
            .headers_mut()
            .insert("Idempotency-Key", key.parse().unwrap());
    }
    request
}

async fn documents(h: &Harness) -> i64 {
    match &h.database {
        Database::Sqlite(pool) => sqlx::query_scalar("SELECT COUNT(*) FROM documents")
            .fetch_one(pool)
            .await
            .unwrap(),
        Database::Postgres(pool) => sqlx::query_scalar("SELECT COUNT(*) FROM documents")
            .fetch_one(pool)
            .await
            .unwrap(),
    }
}

async fn id(response: axum::response::Response) -> String {
    let body: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
    body["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_repeated_api_upload_with_the_same_key_returns_the_first_document() {
    let h = harness().await;
    let key = Uuid::new_v4().to_string();

    let first = h
        .app
        .clone()
        .oneshot(upload("/api/documents/upload", Some(&key), "pixels"))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first = id(first).await;
    let replay = h
        .app
        .clone()
        .oneshot(upload("/api/documents/upload", Some(&key), "pixels"))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);

    assert_eq!(id(replay).await, first);
    assert_eq!(documents(&h).await, 1);
}

#[tokio::test]
async fn a_key_reused_for_different_content_is_a_conflict() {
    let h = harness().await;
    let key = Uuid::new_v4().to_string();
    let first = h
        .app
        .clone()
        .oneshot(upload("/api/documents/upload", Some(&key), "pixels"))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);

    let other = h
        .app
        .clone()
        .oneshot(upload("/api/documents/upload", Some(&key), "other pixels"))
        .await
        .unwrap();

    assert_eq!(other.status(), StatusCode::CONFLICT);
    assert!(text(other).await.contains("Idempotency-Key"));
    assert_eq!(documents(&h).await, 1);
}

#[tokio::test]
async fn uploads_without_a_key_are_not_deduplicated() {
    let h = harness().await;
    for _ in 0..2 {
        let response = h
            .app
            .clone()
            .oneshot(upload("/api/documents/upload", None, "pixels"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert_eq!(documents(&h).await, 2);
}

#[tokio::test]
async fn a_malformed_key_is_rejected_before_anything_is_stored() {
    let h = harness().await;
    for key in ["not-a-uuid", "00000000-0000-0000-0000-000000000000"] {
        let response = h
            .app
            .clone()
            .oneshot(upload("/api/documents/upload", Some(key), "pixels"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{key}");
    }
    assert_eq!(documents(&h).await, 0);
}

#[tokio::test]
async fn the_web_upload_honours_the_key_too() {
    let h = harness().await;
    let key = Uuid::new_v4().to_string();
    for _ in 0..2 {
        let response = h
            .app
            .clone()
            .oneshot(upload("/documents/upload", Some(&key), "pixels"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    assert_eq!(documents(&h).await, 1);
}

#[tokio::test]
async fn a_key_whose_document_was_deleted_is_gone() {
    let h = harness().await;
    let key = Uuid::new_v4().to_string();
    let first = h
        .app
        .clone()
        .oneshot(upload("/api/documents/upload", Some(&key), "pixels"))
        .await
        .unwrap();
    let id: Uuid = id(first).await.parse().unwrap();
    h.state.document_repo.delete(h.vault, id).await.unwrap();

    let replay = h
        .app
        .clone()
        .oneshot(upload("/api/documents/upload", Some(&key), "pixels"))
        .await
        .unwrap();

    assert_eq!(replay.status(), StatusCode::GONE);
}

#[tokio::test]
async fn a_web_replay_returns_the_row_again() {
    let h = harness().await;
    let key = Uuid::new_v4().to_string();
    let first = text(
        h.app
            .clone()
            .oneshot(upload("/documents/upload", Some(&key), "pixels"))
            .await
            .unwrap(),
    )
    .await;
    assert!(first.contains("turbo-stream"));

    let replay = h
        .app
        .clone()
        .oneshot(upload("/documents/upload", Some(&key), "pixels"))
        .await
        .unwrap();

    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(
        text(replay).await,
        first,
        "the lost answer is sent again; Turbo replaces the row it shows"
    );
    assert_eq!(documents(&h).await, 1);
}

#[tokio::test]
async fn a_second_file_part_is_rejected() {
    let h = harness().await;
    let mut request = multipart(&[
        ("file", Some("a.png"), "one"),
        ("file", Some("b.png"), "two"),
    ]);
    *request.uri_mut() = "/api/documents/upload".parse().unwrap();

    let response = h.app.clone().oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(documents(&h).await, 0);
}

/// Holds the first `put` until released, so a test can act while an upload is storing its original.
struct GatedStore {
    inner: folio_infra::ObjectStoreDocumentStore,
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
    first: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl folio_core::ports::DocumentStore for GatedStore {
    fn backend(&self) -> &'static str {
        "gated"
    }
    async fn put(&self, key: &str, data: bytes::Bytes) -> folio_core::Result<()> {
        if self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.started.notify_one();
            self.release.notified().await;
        }
        self.inner.put(key, data).await
    }
    async fn get(&self, key: &str) -> folio_core::Result<folio_core::ports::StoredObject> {
        self.inner.get(key).await
    }
    async fn size(&self, key: &str) -> folio_core::Result<Option<u64>> {
        self.inner.size(key).await
    }
    async fn exists(&self, key: &str) -> folio_core::Result<bool> {
        self.inner.exists(key).await
    }
    async fn delete(&self, key: &str) -> folio_core::Result<()> {
        self.inner.delete(key).await
    }
    async fn delete_prefix(&self, prefix: &str) -> folio_core::Result<u64> {
        self.inner.delete_prefix(prefix).await
    }
    async fn list(&self, prefix: &str) -> folio_core::Result<Vec<String>> {
        self.inner.list(prefix).await
    }
}

fn gated() -> std::sync::Arc<GatedStore> {
    std::sync::Arc::new(GatedStore {
        inner: folio_infra::ObjectStoreDocumentStore::in_memory(),
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        first: std::sync::atomic::AtomicBool::new(true),
    })
}

async fn stored_documents(h: &Harness) -> Vec<folio_core::models::Document> {
    let mut stored = Vec::new();
    for id in h.state.document_repo.list_ids(h.vault, None).await.unwrap() {
        if let Some(document) = h.state.document_repo.find_by_id(h.vault, id).await.unwrap() {
            if document.file_path.is_some() {
                stored.push(document);
            }
        }
    }
    stored
}

#[tokio::test]
async fn an_upload_whose_client_disconnects_still_finishes_and_its_retry_replays_it() {
    let store = gated();
    let h = harness_storing_in(store.clone()).await;
    let key = Uuid::new_v4().to_string();

    let request = tokio::spawn(h.app.clone().oneshot(upload(
        "/api/documents/upload",
        Some(&key),
        "pixels",
    )));
    store.started.notified().await;
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    store.release.notify_one();

    let mut stored = Vec::new();
    for _ in 0..100 {
        stored = stored_documents(&h).await;
        if !stored.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(stored.len(), 1, "the import finished without its request");

    let retry = h
        .app
        .clone()
        .oneshot(upload("/api/documents/upload", Some(&key), "pixels"))
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::OK);
    assert_eq!(id(retry).await, stored[0].id.to_string());
    assert_eq!(documents(&h).await, 1);
}

#[tokio::test]
async fn a_retry_while_the_first_upload_is_storing_says_when_to_try_again() {
    let store = gated();
    let h = harness_storing_in(store.clone()).await;
    let key = Uuid::new_v4().to_string();
    let first = tokio::spawn(h.app.clone().oneshot(upload(
        "/api/documents/upload",
        Some(&key),
        "pixels",
    )));
    store.started.notified().await;

    let retry = h
        .app
        .clone()
        .oneshot(upload("/api/documents/upload", Some(&key), "pixels"))
        .await
        .unwrap();

    assert_eq!(retry.status(), StatusCode::CONFLICT);
    assert!(retry.headers().get("retry-after").is_some());
    store.release.notify_one();
    assert_eq!(first.await.unwrap().unwrap().status(), StatusCode::OK);
    assert_eq!(documents(&h).await, 1);
}
