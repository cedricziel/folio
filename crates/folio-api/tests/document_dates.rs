mod common;

use axum::{body::Body, http::Request};
use folio_core::models::CreateDocument;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn date_patch_distinguishes_omission_null_and_invalid_dates() {
    let h = common::harness().await;
    let document = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Letter".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let path = format!("/api/documents/{}", document.id);
    for (body, status, expected) in [
        (r#"{"document_date":"2000-02-29"}"#, 200, Some("2000-02-29")),
        ("{}", 200, Some("2000-02-29")),
        (r#"{"document_date":"1900-02-29"}"#, 422, Some("2000-02-29")),
        (r#"{"document_date":null}"#, 200, None),
        (
            r#"{"document_date":"2026-11-02","updated_by":"forged"}"#,
            422,
            None,
        ),
    ] {
        let response = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(&path)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{body}");
        let saved = h
            .state
            .document_repo
            .find_by_id(h.vault, document.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.document_date.map(|date| date.to_string()).as_deref(),
            expected
        );
        assert_eq!(saved.created_at, document.created_at);
    }
    let response = h.app.clone().oneshot(common::get(&path)).await.unwrap();
    let json: Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(json["document_date"], Value::Null);
}

#[tokio::test]
async fn web_date_edit_and_clear_preserve_vault_redirect() {
    let h = common::harness().await;
    let document = h
        .state
        .document_repo
        .create(
            h.vault,
            CreateDocument {
                title: "Letter".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let path = format!("/vaults/{}/documents/{}/date", h.vault, document.id);
    for body in [
        "document_date=2026-11-02",
        "document_date=2026-11-02&clear=true",
    ] {
        let response = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&path)
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert_eq!(
            response.headers()["location"],
            format!("/vaults/{}/documents/{}", h.vault, document.id)
        );
    }
    assert_eq!(
        h.state
            .document_repo
            .find_by_id(h.vault, document.id)
            .await
            .unwrap()
            .unwrap()
            .document_date,
        None
    );
}
