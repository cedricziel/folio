mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use folio_auth::crypto::hash_token;
use folio_auth::{Session, User};
use folio_core::models::{
    CreateDocument, Document, Principal, UpdateDocument, UserId, VaultId, VaultRole,
};
use folio_core::{DocumentEvent, EventOrigin, OutboxEvent};
use http_body_util::BodyExt;
use tower::ServiceExt;

use common::{harness_requiring_login, multipart, text, Harness};

struct SignedIn {
    user: UserId,
    vault: VaultId,
    cookie: String,
}

async fn sign_in(h: &Harness, name: &str) -> SignedIn {
    h.auth_store
        .create_user(User {
            id: name.into(),
            username: name.into(),
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
    let token = format!("vault-test-session-{name}");
    h.auth_store
        .create_session(Session {
            id_hash: hash_token(&token),
            user_id: name.into(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            created_at: 1,
            user_agent: Some("vault regression".into()),
        })
        .await
        .unwrap();
    let user = UserId::from(name);
    let vault = h.state.vaults.provision_user(&user, false).await.unwrap();
    SignedIn {
        user,
        vault,
        cookie: format!("folio_session={token}"),
    }
}

fn request(method: &str, path: &str, signed_in: &SignedIn, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://localhost")
        .header(header::COOKIE, &signed_in.cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .unwrap()
}

fn broadcast(h: &Harness, id: i64, event: DocumentEvent) {
    h.state.event_broadcaster.send(OutboxEvent {
        id,
        event,
        origin: EventOrigin::User,
        created_at: chrono::Utc::now(),
        attempts: 1,
    });
}

async fn stream_until(body: &mut Body, marker: &str) -> String {
    let mut text = String::new();
    while !text.contains(marker) {
        let frame = tokio::time::timeout(Duration::from_secs(2), body.frame())
            .await
            .expect("the visible event arrives before the deadline")
            .expect("the event stream stays open")
            .unwrap();
        if let Some(data) = frame.data_ref() {
            text.push_str(&String::from_utf8_lossy(data));
        }
    }
    text
}

#[tokio::test]
async fn authenticated_text_creation_records_the_uploader_and_default_vault() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "text-creator").await;

    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            "/api/documents",
            &alice,
            Body::from(r#"{"title":"Created text","content":"Document content"}"#),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let created: Document = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(created.vault_id, alice.vault);
    assert_eq!(created.uploaded_by, Some(alice.user.clone()));
    let stored = h
        .state
        .document_repo
        .find_by_id(alice.vault, created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.uploaded_by, Some(alice.user));
    assert!(h
        .state
        .document_repo
        .find_by_id(h.vault, created.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn authenticated_file_upload_records_the_uploader_and_default_vault() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "file-uploader").await;
    let mut upload = multipart(&[
        ("title", None, "Uploaded file"),
        ("file", Some("scan.png"), "scan bytes"),
    ]);
    *upload.uri_mut() = "/api/documents/upload".parse().unwrap();
    upload
        .headers_mut()
        .insert(header::COOKIE, alice.cookie.parse().unwrap());
    upload
        .headers_mut()
        .insert(header::ORIGIN, "http://localhost".parse().unwrap());
    upload
        .headers_mut()
        .insert(header::HOST, "localhost".parse().unwrap());

    let response = h.app.clone().oneshot(upload).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let created: Document = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(created.vault_id, alice.vault);
    assert_eq!(created.uploaded_by, Some(alice.user.clone()));
    let stored = h
        .state
        .document_repo
        .find_by_id(alice.vault, created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.uploaded_by, Some(alice.user));
    assert!(stored.file_path.is_some());
    assert!(h
        .state
        .document_repo
        .find_by_id(h.vault, created.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn another_users_default_vault_documents_are_hidden_from_document_and_file_routes() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "hidden-reader").await;
    let bob = sign_in(&h, "hidden-owner").await;
    let secret = h
        .state
        .document_repo
        .create(
            bob.vault,
            CreateDocument {
                title: "Private document".into(),
                mime_type: Some("application/pdf".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let key = folio_core::ports::keys::original(secret.id);
    h.state
        .store
        .put(&key, b"private PDF".to_vec().into())
        .await
        .unwrap();
    h.state
        .document_repo
        .set_file_path(bob.vault, secret.id, &key)
        .await
        .unwrap();

    let document = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/documents/{}", secret.id),
            &alice,
            Body::empty(),
        ))
        .await
        .unwrap();
    let download = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/documents/{}/download", secret.id),
            &alice,
            Body::empty(),
        ))
        .await
        .unwrap();
    let thumbnail = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/documents/{}/thumbnail", secret.id),
            &alice,
            Body::empty(),
        ))
        .await
        .unwrap();
    let page = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/documents/{}/page/1/thumbnail", secret.id),
            &alice,
            Body::empty(),
        ))
        .await
        .unwrap();

    assert_eq!(document.status(), StatusCode::NOT_FOUND);
    assert_eq!(download.status(), StatusCode::NOT_FOUND);
    assert_eq!(thumbnail.status(), StatusCode::NOT_FOUND);
    assert_eq!(page.status(), StatusCode::NOT_FOUND);
    assert!(!text(download).await.contains("private PDF"));
}

#[tokio::test]
async fn configured_ingestion_sources_are_visible_only_in_their_import_vault() {
    let source_id = uuid::Uuid::new_v4();
    let source = folio_core::models::IngestionSourceDef {
        id: source_id,
        vault_id: None,
        name: "Instance consume folder".into(),
        enabled: true,
        origin: folio_core::models::SourceOrigin::Config,
        config: folio_core::models::SourceConfig::Folder(
            serde_json::from_value(serde_json::json!({"path":"/private-instance-consume"}))
                .unwrap(),
        ),
    };
    let h = common::harness_with_sources(vec![source]).await;
    let alice = sign_in(&h, "source-reader").await;
    let visible = h
        .app
        .clone()
        .oneshot(common::get("/api/sources"))
        .await
        .unwrap();
    assert_eq!(visible.status(), StatusCode::OK);
    assert!(text(visible).await.contains("Instance consume folder"));
    let hidden = h
        .app
        .clone()
        .oneshot(request("GET", "/api/sources", &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::OK);
    assert_eq!(text(hidden).await, "[]");
    let unknown = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/sources/{source_id}"),
            &alice,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn event_stream_drops_events_from_unrelated_vaults() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "stream-reader").await;
    let bob = sign_in(&h, "stream-secret-owner").await;
    let visible = h
        .state
        .document_repo
        .create(
            alice.vault,
            CreateDocument {
                title: "Visible stream document".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let secret = h
        .state
        .document_repo
        .create(
            bob.vault,
            CreateDocument {
                title: "Secret stream document".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let response = h
        .app
        .clone()
        .oneshot(request("GET", "/events", &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();

    broadcast(
        &h,
        100,
        DocumentEvent::Created {
            document_id: secret.id,
        },
    );
    broadcast(
        &h,
        101,
        DocumentEvent::Created {
            document_id: visible.id,
        },
    );
    let delivered = stream_until(&mut body, "Visible stream document").await;

    assert!(delivered.contains(&visible.id.to_string()));
    assert!(!delivered.contains(&secret.id.to_string()));
    assert!(!delivered.contains("Secret stream document"));
}

#[tokio::test]
async fn event_stream_closes_before_delivering_events_after_membership_is_revoked() {
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "stream-vault-owner").await;
    let viewer = sign_in(&h, "stream-revoked-viewer").await;
    h.members
        .grant(
            owner.vault,
            &Principal::User(viewer.user.clone()),
            VaultRole::Viewer,
            Some(&owner.user),
        )
        .await
        .unwrap();
    h.vault_repository
        .set_default(&viewer.user, owner.vault)
        .await
        .unwrap();
    let visible = h
        .state
        .document_repo
        .create(
            owner.vault,
            CreateDocument {
                title: "Before revocation".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let response = h
        .app
        .clone()
        .oneshot(request("GET", "/events", &viewer, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    broadcast(
        &h,
        200,
        DocumentEvent::Created {
            document_id: visible.id,
        },
    );
    stream_until(&mut body, "Before revocation").await;

    h.members
        .revoke(owner.vault, &Principal::User(viewer.user))
        .await
        .unwrap();
    h.state
        .document_repo
        .update(
            owner.vault,
            visible.id,
            UpdateDocument {
                title: Some("After revocation secret".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    broadcast(
        &h,
        201,
        DocumentEvent::Updated {
            document_id: visible.id,
        },
    );
    let next = tokio::time::timeout(Duration::from_secs(2), body.frame())
        .await
        .expect("revocation closes the stream when the next event arrives");

    assert!(
        next.is_none(),
        "a revoked subscriber must receive no event frame"
    );
}
