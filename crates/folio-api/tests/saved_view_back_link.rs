mod common;
use axum::{
    body::Body,
    http::{header, Request},
};
use folio_auth::{crypto::hash_token, Session, User};
use folio_core::models::{CreateDocument, UserId};
use tower::ServiceExt;

fn send(method: &str, path: String, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://localhost")
        .header(header::COOKIE, "folio_session=back-link-session")
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .unwrap()
}

#[tokio::test]
async fn saved_view_rows_lead_back_to_the_saved_view() {
    let h = common::harness_requiring_login().await;
    h.auth_store
        .create_user(User {
            id: "alice".into(),
            username: "alice".into(),
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
            id_hash: hash_token("back-link-session"),
            user_id: "alice".into(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    let vault = h
        .state
        .vaults
        .provision_user(&UserId::from("alice"))
        .await
        .unwrap();
    let document = h
        .state
        .document_repo
        .create(
            vault,
            CreateDocument {
                title: "In a saved view".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let body = r#"{"name":"Everything","query":{"version":1,"filters":{"created_after":"2000-01-01T00:00:00Z"}}}"#;
    let created = h
        .app
        .clone()
        .oneshot(send(
            "POST",
            format!("/api/vaults/{vault}/saved-views"),
            Body::from(body),
        ))
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_str(&common::text(created).await).unwrap();
    let view = created["id"].as_str().unwrap();

    let list = format!("/vaults/{vault}/saved-views/{view}");
    let html = common::text(
        h.app
            .clone()
            .oneshot(send("GET", list, Body::empty()))
            .await
            .unwrap(),
    )
    .await;
    let expected = format!(
        "/documents/{}?return_to=%2Fsaved-views%2F{view}",
        document.id
    );
    assert!(html.contains(&expected), "{expected}");

    let opened = format!(
        "/vaults/{vault}/documents/{}?return_to=%2Fsaved-views%2F{view}",
        document.id
    );
    let page = common::text(
        h.app
            .clone()
            .oneshot(send("GET", opened, Body::empty()))
            .await
            .unwrap(),
    )
    .await;
    assert!(page.contains("Back to saved view"));
}
