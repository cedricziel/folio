mod common;

use axum::{body::Body, http::Request};
use folio_core::{models::CreateDocument, ports::keys};
use tower::ServiceExt;

fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn deletion_keeps_files_until_explicit_purge_and_restore_recovers_document() {
    let h = common::harness().await;
    let doc = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Recoverable invoice".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let key = keys::original(doc.id);
    h.state
        .store
        .put(&key, "original contents".into())
        .await
        .unwrap();
    let path = format!("/api/documents/{}", doc.id);
    let response = h
        .app
        .clone()
        .oneshot(request("DELETE", &path))
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    assert!(h.state.store.exists(&key).await.unwrap());
    assert_eq!(
        h.app
            .clone()
            .oneshot(common::get(&path))
            .await
            .unwrap()
            .status(),
        404
    );
    let trash = h
        .app
        .clone()
        .oneshot(common::get("/api/documents/trash"))
        .await
        .unwrap();
    assert_eq!(trash.status(), 200);
    assert!(common::text(trash).await.contains(&doc.id.to_string()));
    let response = h
        .app
        .clone()
        .oneshot(request("POST", &format!("{path}/restore")))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        h.app
            .clone()
            .oneshot(common::get(&path))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(request("DELETE", &format!("{path}/purge")))
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(request("DELETE", &path))
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(request("DELETE", &format!("{path}/purge")))
            .await
            .unwrap()
            .status(),
        202
    );
    assert!(h.state.store.exists(&key).await.unwrap());
    assert_eq!(
        h.app
            .clone()
            .oneshot(request("POST", &format!("{path}/restore")))
            .await
            .unwrap()
            .status(),
        409
    );
    let trash = common::text(h.app.clone().oneshot(common::get("/trash")).await.unwrap()).await;
    assert!(trash.contains("Deleting permanently"));
    assert!(!trash.contains(&format!("/documents/{}/restore", doc.id)));
    assert!(!trash.contains(&format!("/documents/{}/delete-forever", doc.id)));
}
