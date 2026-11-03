mod common;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use folio_auth::{crypto::hash_token, Session, User};
use folio_core::models::{CreateDocument, Principal, UserId, VaultRole};
use folio_core::mutation::with_actor;
use tower::ServiceExt;

#[tokio::test]
async fn authenticated_document_mutations_ignore_forged_actor_fields() {
    let h = common::harness().await;
    h.auth_store
        .create_user(User {
            id: "document-actor".into(),
            username: "document-actor".into(),
            password_hash: "!".into(),
            created_at: 1,
            email: None,
            display_name: None,
            is_admin: false,
            is_owner: false,
            auth_source: "local".into(),
            external_id: None,
        })
        .await
        .unwrap();
    h.auth_store
        .create_session(Session {
            id_hash: hash_token("document-session"),
            user_id: "document-actor".into(),
            expires_at: i64::MAX / 2,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    h.members
        .grant(
            h.vault,
            &Principal::User(UserId::from("document-actor")),
            VaultRole::Editor,
            None,
        )
        .await
        .unwrap();
    let response = h.app.clone().oneshot(Request::post("/api/documents")
        .header(header::COOKIE, "folio_session=document-session")
        .header("X-Folio-Vault", h.vault.to_string())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"title":"Audit history","created_by":"forged","updated_by":"forged","uploaded_by":"forged"}"#)).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let document: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(document["created_by"], "document-actor");
    assert_eq!(document["updated_by"], "document-actor");
    assert_eq!(document["uploaded_by"], "document-actor");
    let id = document["id"].as_str().unwrap();
    let response = h
        .app
        .clone()
        .oneshot(
            Request::delete(format!("/api/documents/{id}"))
                .header(header::COOKIE, "folio_session=document-session")
                .header("X-Folio-Vault", h.vault.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let trashed = h
        .state
        .document_repo
        .find_trashed_by_id(h.vault, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(trashed.updated_by, Some(UserId::from("document-actor")));
    let response = h
        .app
        .clone()
        .oneshot(
            Request::post(format!("/api/documents/{id}/restore"))
                .header(header::COOKIE, "folio_session=document-session")
                .header("X-Folio-Vault", h.vault.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let restored: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(restored["created_by"], "document-actor");
    assert_eq!(restored["updated_by"], "document-actor");
    assert_eq!(restored["created_at"], document["created_at"]);
}

#[tokio::test]
async fn document_detail_history_shows_server_actor_ids_and_unknown_system_updates() {
    let h = common::harness().await;
    let document = with_actor(
        Some(UserId::from("archive-creator")),
        h.state.document_repo.create(
            h.vault,
            CreateDocument {
                title: "History detail".into(),
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap();
    with_actor(
        None,
        h.state
            .document_repo
            .set_page_count(h.vault, document.id, 3),
    )
    .await
    .unwrap();
    let response = h
        .app
        .clone()
        .oneshot(common::get(&format!("/documents/{}", document.id)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = common::text(response).await;
    assert!(html.contains("History"));
    assert!(html.contains("archive-creator"));
    assert!(html.contains("System or unknown"));
}
