mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{harness, multipart, text, Harness};
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
async fn a_web_replay_does_not_add_the_row_again() {
    let h = harness().await;
    let key = Uuid::new_v4().to_string();
    let first = h
        .app
        .clone()
        .oneshot(upload("/documents/upload", Some(&key), "pixels"))
        .await
        .unwrap();
    assert!(text(first).await.contains("turbo-stream"));

    let replay = h
        .app
        .clone()
        .oneshot(upload("/documents/upload", Some(&key), "pixels"))
        .await
        .unwrap();

    assert_eq!(replay.status(), StatusCode::OK);
    assert!(!text(replay).await.contains("turbo-stream"));
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
