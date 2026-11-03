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
    let vault = h.state.vaults.provision_user(&user).await.unwrap();
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

async fn captured(h: &Harness, vault: VaultId, event: &DocumentEvent) -> OutboxEvent {
    let id = h
        .state
        .outbox
        .append_document(vault, event, EventOrigin::User)
        .await
        .unwrap();
    h.state
        .outbox
        .replay(id - 1, chrono::Utc::now() - chrono::Duration::days(1), 1)
        .await
        .unwrap()
        .pop()
        .unwrap()
}

async fn broadcast(h: &Harness, vault: VaultId, id: i64, event: DocumentEvent) {
    let mut event = captured(h, vault, &event).await;
    event.id = id;
    h.state.event_broadcaster.send(event);
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
        bob.vault,
        100,
        DocumentEvent::Created {
            document_id: secret.id,
        },
    )
    .await;
    broadcast(
        &h,
        alice.vault,
        101,
        DocumentEvent::Created {
            document_id: visible.id,
        },
    )
    .await;
    let delivered = stream_until(&mut body, "Visible stream document").await;

    assert!(delivered.contains(&visible.id.to_string()));
    assert!(!delivered.contains(&secret.id.to_string()));
    assert!(!delivered.contains("Secret stream document"));
}

#[tokio::test]
async fn event_stream_suppresses_restricted_domain_envelopes_with_document_projections() {
    use folio_core::{DomainEventEnvelope, DomainEventSubject, EventAudience};
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "restricted-stream-reader").await;
    let visible = h
        .state
        .document_repo
        .create(
            alice.vault,
            CreateDocument {
                title: "Visible public transition".into(),
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
    let mut body = response.into_body();
    let event = DocumentEvent::Updated {
        document_id: visible.id,
    };
    let context = captured(&h, alice.vault, &event).await.context;
    let envelope = DomainEventEnvelope::new(
        "document.updated",
        alice.vault,
        DomainEventSubject {
            subject_type: "document".into(),
            id: visible.id,
        },
        Some(visible.id),
        EventAudience::User {
            user_id: UserId::from("private-owner"),
        },
        serde_json::json!({}),
    )
    .unwrap();
    h.state.event_broadcaster.send(OutboxEvent {
        id: 300,
        event: Some(event),
        envelope: Some(envelope),
        origin: EventOrigin::User,
        context: context.clone(),
        created_at: chrono::Utc::now(),
        attempts: 1,
    });
    let owners_envelope = DomainEventEnvelope::new(
        "document.updated",
        alice.vault,
        DomainEventSubject {
            subject_type: "document".into(),
            id: visible.id,
        },
        Some(visible.id),
        EventAudience::Owners,
        serde_json::json!({}),
    )
    .unwrap();
    h.state.event_broadcaster.send(OutboxEvent {
        id: 301,
        event: Some(DocumentEvent::Updated {
            document_id: visible.id,
        }),
        envelope: Some(owners_envelope),
        origin: EventOrigin::User,
        context,
        created_at: chrono::Utc::now(),
        attempts: 1,
    });
    broadcast(
        &h,
        alice.vault,
        302,
        DocumentEvent::Updated {
            document_id: visible.id,
        },
    )
    .await;
    let delivered = stream_until(&mut body, "Visible public transition").await;
    assert!(!delivered.contains("id: 300\n"));
    assert!(!delivered.contains("id: 301\n"));
    assert!(delivered.contains("id: 302\n"));
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
        owner.vault,
        200,
        DocumentEvent::Created {
            document_id: visible.id,
        },
    )
    .await;
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
        owner.vault,
        201,
        DocumentEvent::Updated {
            document_id: visible.id,
        },
    )
    .await;
    let next = tokio::time::timeout(Duration::from_secs(2), body.frame())
        .await
        .expect("revocation closes the stream when the next event arrives");

    assert!(
        next.is_none(),
        "a revoked subscriber must receive no event frame"
    );
}

#[tokio::test]
async fn browse_pages_are_bounded_and_recheck_vault_access() {
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "browse-owner").await;
    let viewer = sign_in(&h, "browse-viewer").await;
    for title in ["A", "B", "C"] {
        h.state
            .document_repo
            .create(
                owner.vault,
                CreateDocument {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    let path = format!("/api/vaults/{}/documents?limit=2&sort=title", owner.vault);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &viewer, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let invisible = text(response).await;
    let unknown = format!("/api/vaults/{}/documents", VaultId::new());
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &unknown, &viewer, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(text(response).await, invisible);
    h.members
        .grant(
            owner.vault,
            &Principal::User(viewer.user.clone()),
            VaultRole::Viewer,
            Some(&owner.user),
        )
        .await
        .unwrap();
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &viewer, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let first: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(first["documents"].as_array().unwrap().len(), 2);
    assert_eq!(first["documents"][0]["title"], "A");
    let cursor = first["next_cursor"].as_str().unwrap();
    let next = format!("{path}&cursor={cursor}");
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &next, &viewer, Body::empty()))
        .await
        .unwrap();
    let second: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(second["documents"].as_array().unwrap().len(), 1);
    assert_eq!(second["documents"][0]["title"], "C");
    assert!(second["next_cursor"].is_null());
    for suffix in ["limit=0", "limit=101", "sort=invalid", "cursor=garbage"] {
        let uri = format!("/api/documents/browse?{suffix}");
        let response = h
            .app
            .clone()
            .oneshot(request("GET", &uri, &owner, Body::empty()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{suffix}");
    }
    let mismatch = format!("/api/documents/browse?sort=oldest&cursor={cursor}");
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &mismatch, &owner, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    h.members
        .revoke(owner.vault, &Principal::User(viewer.user.clone()))
        .await
        .unwrap();
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &next, &viewer, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", "/api/documents", &owner, Body::empty()))
        .await
        .unwrap();
    let legacy: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(legacy["count"], 3);
}

#[tokio::test]
async fn default_browse_keeps_filters_and_enrichment_with_paging_parameters() {
    let h = common::harness().await;
    let tag = h
        .state
        .tag_repo
        .create(
            h.vault,
            folio_core::models::CreateTag {
                name: "Insurance".into(),
                color: Some("#123456".into()),
            },
        )
        .await
        .unwrap();
    for title in ["Tagged A", "Tagged B", "Other"] {
        let doc = h
            .state
            .document_repo
            .create(
                h.vault,
                CreateDocument {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        if title.starts_with("Tagged") {
            h.state
                .document_repo
                .add_tag(h.vault, doc.id, tag.id)
                .await
                .unwrap();
        }
    }
    let uri = format!("/api/documents/browse?tag_id={}&limit=1&sort=title", tag.id);
    let response = h.app.clone().oneshot(common::get(&uri)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let first: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(first["documents"][0]["title"], "Tagged A");
    let next = format!("{uri}&cursor={}", first["next_cursor"].as_str().unwrap());
    let response = h.app.clone().oneshot(common::get(&next)).await.unwrap();
    let second: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(second["documents"][0]["title"], "Tagged B");
    assert_eq!(second["documents"][0]["tags"][0]["name"], "Insurance");
    assert!(second["next_cursor"].is_null());
    let foreign = h.vault_repository.create("Private", None).await.unwrap();
    let response = h
        .app
        .clone()
        .oneshot(common::get(&format!(
            "/api/vaults/{}/documents",
            foreign.id
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let secured = harness_requiring_login().await;
    let response = secured
        .app
        .clone()
        .oneshot(common::get("/api/documents/browse"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn browse_search_combines_text_with_all_tags_before_paging() {
    let h = common::harness().await;
    let mut tags = Vec::new();
    for name in ["Tax", "Home"] {
        tags.push(
            h.state
                .tag_repo
                .create(
                    h.vault,
                    folio_core::models::CreateTag {
                        name: name.into(),
                        color: None,
                    },
                )
                .await
                .unwrap(),
        );
    }
    for (title, content, both) in [
        ("A unrelated", "Other content", true),
        ("B insurance", "Single tag", false),
        ("C matching", "Insurance policy", true),
        ("D insurance", "Second match", true),
    ] {
        let doc = h
            .state
            .document_repo
            .create(
                h.vault,
                CreateDocument {
                    title: title.into(),
                    content: Some(content.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        h.state
            .document_repo
            .add_tag(h.vault, doc.id, tags[0].id)
            .await
            .unwrap();
        if both {
            h.state
                .document_repo
                .add_tag(h.vault, doc.id, tags[1].id)
                .await
                .unwrap();
        }
    }
    let uri = format!(
        "/api/documents/browse?q=insurance&tag_id={}&tag_id={}&limit=1&sort=title",
        tags[0].id, tags[1].id
    );
    let response = h.app.clone().oneshot(common::get(&uri)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let first: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(first["documents"][0]["title"], "C matching");
    let cursor = first["next_cursor"].as_str().unwrap();
    let response = h
        .app
        .clone()
        .oneshot(common::get(&format!("{uri}&cursor={cursor}")))
        .await
        .unwrap();
    let second: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(second["documents"][0]["title"], "D insurance");
    assert!(second["next_cursor"].is_null());
    let changed = format!("{}&cursor={cursor}", uri.replace("q=insurance", "q=policy"));
    let response = h.app.clone().oneshot(common::get(&changed)).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn browse_enforces_the_search_term_limit() {
    let h = common::harness().await;
    for (words, expected) in [
        (32, StatusCode::OK),
        (33, StatusCode::BAD_REQUEST),
        (1000, StatusCode::BAD_REQUEST),
    ] {
        let query = vec!["word"; words].join("+");
        let response = h
            .app
            .clone()
            .oneshot(common::get(&format!("/api/documents/browse?q={query}")))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[tokio::test]
async fn discovery_lists_accessible_vaults_and_the_default() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "discover-alice").await;
    let bob = sign_in(&h, "discover-bob").await;
    let response = h
        .app
        .clone()
        .oneshot(request("GET", "/api/vaults", &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(body["default_vault_id"], alice.vault.to_string());
    assert_eq!(body["vaults"].as_array().unwrap().len(), 1);
    assert_eq!(body["vaults"][0]["id"], alice.vault.to_string());
    assert_eq!(body["vaults"][0]["role"], "owner");
    assert!(!body.to_string().contains(&bob.vault.to_string()));
}

#[tokio::test]
async fn explicit_vault_upload_uses_selected_scope_without_changing_default() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "select-alice").await;
    let bob = sign_in(&h, "select-bob").await;
    h.members
        .grant(
            bob.vault,
            &Principal::User(alice.user.clone()),
            VaultRole::Contributor,
            None,
        )
        .await
        .unwrap();
    let mut req = request(
        "POST",
        "/api/documents",
        &alice,
        Body::from(r#"{"title":"Shared file","content":"Shared content"}"#),
    );
    req.headers_mut()
        .insert("x-folio-vault", bob.vault.to_string().parse().unwrap());
    let response = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let created: Document = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(created.vault_id, bob.vault);
    assert_eq!(created.uploaded_by, Some(alice.user.clone()));
    assert_eq!(
        h.vault_repository
            .default_for_user(&alice.user)
            .await
            .unwrap(),
        Some(alice.vault)
    );
    assert!(h
        .state
        .document_repo
        .find_by_id(alice.vault, created.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn invisible_vault_has_the_same_response_as_unknown_vault() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "hidden-alice").await;
    let bob = sign_in(&h, "hidden-bob").await;
    let mut hidden = request("GET", "/api/documents", &alice, Body::empty());
    hidden
        .headers_mut()
        .insert("x-folio-vault", bob.vault.to_string().parse().unwrap());
    let hidden = h.app.clone().oneshot(hidden).await.unwrap();
    let mut unknown = request("GET", "/api/documents", &alice, Body::empty());
    unknown
        .headers_mut()
        .insert("x-folio-vault", VaultId::new().to_string().parse().unwrap());
    let unknown = h.app.clone().oneshot(unknown).await.unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert_eq!(text(hidden).await, text(unknown).await);
}

#[tokio::test]
async fn selected_viewer_cannot_create_documents_or_vocabulary() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "viewer-alice").await;
    let bob = sign_in(&h, "viewer-bob").await;
    h.members
        .grant(
            bob.vault,
            &Principal::User(alice.user.clone()),
            VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();
    let mut req = request(
        "POST",
        "/api/documents",
        &alice,
        Body::from(r#"{"title":"Denied","content":"Denied"}"#),
    );
    req.headers_mut()
        .insert("x-folio-vault", bob.vault.to_string().parse().unwrap());
    let response = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(h
        .state
        .document_repo
        .list(bob.vault, None)
        .await
        .unwrap()
        .is_empty());
    let mut req = request(
        "POST",
        "/api/tags",
        &alice,
        Body::from(r#"{"name":"Denied"}"#),
    );
    req.headers_mut()
        .insert("x-folio-vault", bob.vault.to_string().parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn scoped_web_navigation_keeps_the_selected_vault() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "web-alice").await;
    let path = format!("/vaults/{}/?view=all", alice.vault);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = text(response).await;
    assert!(html.contains(&format!("/vaults/{}/?view=all", alice.vault)));
    assert!(html.contains("Personal"));
}

#[tokio::test]
async fn malformed_vault_selection_is_not_silently_ignored() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "invalid-selection").await;
    let mut req = request("GET", "/api/documents", &alice, Body::empty());
    req.headers_mut()
        .insert("x-folio-vault", "not-a-uuid".parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn revocation_blocks_the_next_selected_request() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "revoked-alice").await;
    let bob = sign_in(&h, "revoked-bob").await;
    let principal = Principal::User(alice.user.clone());
    h.members
        .grant(bob.vault, &principal, VaultRole::Viewer, None)
        .await
        .unwrap();
    let mut req = request("GET", "/api/documents", &alice, Body::empty());
    req.headers_mut()
        .insert("x-folio-vault", bob.vault.to_string().parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::OK
    );
    h.members.revoke(bob.vault, &principal).await.unwrap();
    let mut req = request("GET", "/api/documents", &alice, Body::empty());
    req.headers_mut()
        .insert("x-folio-vault", bob.vault.to_string().parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn contributor_cannot_delete_another_uploaders_document() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "contributor-alice").await;
    let bob = sign_in(&h, "contributor-bob").await;
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            "/api/documents",
            &bob,
            Body::from(r#"{"title":"Bob's file","content":"Private contribution"}"#),
        ))
        .await
        .unwrap();
    let doc: Document = serde_json::from_str(&text(response).await).unwrap();
    h.members
        .grant(
            bob.vault,
            &Principal::User(alice.user.clone()),
            VaultRole::Contributor,
            None,
        )
        .await
        .unwrap();
    let mut req = request(
        "DELETE",
        &format!("/api/documents/{}", doc.id),
        &alice,
        Body::empty(),
    );
    req.headers_mut()
        .insert("x-folio-vault", bob.vault.to_string().parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert!(h
        .state
        .document_repo
        .find_by_id(bob.vault, doc.id)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn contributor_can_delete_their_own_upload_in_a_shared_vault() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "own-alice").await;
    let bob = sign_in(&h, "own-bob").await;
    h.members
        .grant(
            bob.vault,
            &Principal::User(alice.user.clone()),
            VaultRole::Contributor,
            None,
        )
        .await
        .unwrap();
    let mut create = request(
        "POST",
        "/api/documents",
        &alice,
        Body::from(r#"{"title":"Alice's file","content":"Own contribution"}"#),
    );
    create
        .headers_mut()
        .insert("x-folio-vault", bob.vault.to_string().parse().unwrap());
    let response = h.app.clone().oneshot(create).await.unwrap();
    let doc: Document = serde_json::from_str(&text(response).await).unwrap();
    let mut delete = request(
        "DELETE",
        &format!("/api/documents/{}", doc.id),
        &alice,
        Body::empty(),
    );
    delete
        .headers_mut()
        .insert("x-folio-vault", bob.vault.to_string().parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(delete).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    assert!(h
        .state
        .document_repo
        .find_by_id(bob.vault, doc.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn scoped_web_resource_extracts_document_id_and_keeps_redirect_scope() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "scoped-document").await;
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            "/api/documents",
            &alice,
            Body::from(r#"{"title":"Scoped page","content":"Scoped content"}"#),
        ))
        .await
        .unwrap();
    let doc: Document = serde_json::from_str(&text(response).await).unwrap();
    let path = format!("/vaults/{}/documents/{}", alice.vault, doc.id);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(text(response).await.contains("Scoped page"));
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &format!("{path}/delete"),
            &alice,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()[header::LOCATION],
        format!("/vaults/{}/", alice.vault)
    );
}

#[tokio::test]
async fn login_preserves_the_scoped_deep_link() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "login-link").await;
    let path = format!("/vaults/{}/?view=all", alice.vault);
    let response = h
        .app
        .clone()
        .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers()[header::LOCATION].to_str().unwrap();
    let parsed = url::Url::parse(&format!("http://localhost{location}")).unwrap();
    assert_eq!(parsed.path(), "/login");
    assert_eq!(
        parsed
            .query_pairs()
            .find(|(key, _)| key == "next")
            .unwrap()
            .1,
        path
    );
}

#[tokio::test]
async fn conflicting_url_and_header_vaults_are_rejected() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "conflict-alice").await;
    for path in [
        format!("/vaults/{}/api/documents", alice.vault),
        format!("/api/vaults/{}/documents", alice.vault),
    ] {
        let mut req = request("GET", &path, &alice, Body::empty());
        req.headers_mut()
            .insert("x-folio-vault", VaultId::new().to_string().parse().unwrap());
        assert_eq!(
            h.app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::BAD_REQUEST,
            "{path}"
        );
    }
}

#[tokio::test]
async fn scoped_mutations_still_require_same_origin_for_sessions() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "csrf-scope").await;
    let mut req = request(
        "POST",
        &format!("/vaults/{}/api/documents", alice.vault),
        &alice,
        Body::from(r#"{"title":"Attack","content":"Attack"}"#),
    );
    req.headers_mut()
        .insert(header::ORIGIN, "https://attacker.invalid".parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert!(h
        .state
        .document_repo
        .list(alice.vault, None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn editor_cannot_manage_vault_workflows() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "workflow-editor").await;
    let bob = sign_in(&h, "workflow-owner").await;
    h.members
        .grant(
            bob.vault,
            &Principal::User(alice.user.clone()),
            VaultRole::Editor,
            None,
        )
        .await
        .unwrap();
    let mut req = request("GET", "/api/workflows", &alice, Body::empty());
    req.headers_mut()
        .insert("x-folio-vault", bob.vault.to_string().parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn trash_actions_enforce_vault_and_document_permissions() {
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "trash-owner").await;
    let viewer = sign_in(&h, "trash-viewer").await;
    let outsider = sign_in(&h, "trash-outsider").await;
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
    let doc = h
        .state
        .document_repo
        .create(
            owner.vault,
            CreateDocument {
                title: "Private recoverable document".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let path = format!("/api/documents/{}", doc.id);
    assert_eq!(
        h.app
            .clone()
            .oneshot(request("DELETE", &path, &viewer, Body::empty()))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(request("DELETE", &path, &outsider, Body::empty()))
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(request("DELETE", &path, &owner, Body::empty()))
            .await
            .unwrap()
            .status(),
        204
    );
    for (method, suffix) in [("POST", "restore"), ("DELETE", "purge")] {
        assert_eq!(
            h.app
                .clone()
                .oneshot(request(
                    method,
                    &format!("{path}/{suffix}"),
                    &viewer,
                    Body::empty()
                ))
                .await
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            h.app
                .clone()
                .oneshot(request(
                    method,
                    &format!("{path}/{suffix}"),
                    &outsider,
                    Body::empty()
                ))
                .await
                .unwrap()
                .status(),
            404
        );
    }
    let hidden = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            "/api/documents/trash",
            &outsider,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(hidden.status(), 200);
    assert!(!text(hidden).await.contains(&doc.id.to_string()));
    assert!(folio_api::routes::turbo_streams_for(
        folio_api::vault::DefaultVault(outsider.vault),
        &h.state,
        &DocumentEvent::Trashed {
            document_id: doc.id
        }
    )
    .await
    .unwrap()
    .is_empty());
    assert!(!folio_api::routes::turbo_streams_for(
        folio_api::vault::DefaultVault(owner.vault),
        &h.state,
        &DocumentEvent::Trashed {
            document_id: doc.id
        }
    )
    .await
    .unwrap()
    .is_empty());
}

fn selected_vault_request(
    method: &str,
    path: &str,
    signed_in: &SignedIn,
    vault: VaultId,
    body: Body,
) -> Request<Body> {
    let mut req = request(method, path, signed_in, body);
    req.headers_mut()
        .insert("x-folio-vault", vault.to_string().parse().unwrap());
    req
}

#[tokio::test]
async fn selected_vault_trash_supports_idempotent_delete_restore_and_scoped_forms() {
    let h = harness_requiring_login().await;
    let editor = sign_in(&h, "selected-trash-editor").await;
    let owner = sign_in(&h, "selected-trash-owner").await;
    h.members
        .grant(
            owner.vault,
            &Principal::User(editor.user.clone()),
            VaultRole::Editor,
            None,
        )
        .await
        .unwrap();
    let document = h
        .state
        .document_repo
        .create(
            owner.vault,
            CreateDocument {
                title: "Selected recoverable document".into(),
                uploaded_by: Some(editor.user.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let api = format!("/api/documents/{}", document.id);
    let scoped = format!("/vaults/{}/documents/{}", owner.vault, document.id);
    let trash = format!("/vaults/{}/trash", owner.vault);

    let response = h
        .app
        .clone()
        .oneshot(selected_vault_request(
            "DELETE",
            &api,
            &editor,
            owner.vault,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = h
        .app
        .clone()
        .oneshot(selected_vault_request(
            "DELETE",
            &api,
            &editor,
            owner.vault,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = h
        .app
        .clone()
        .oneshot(selected_vault_request(
            "PUT",
            &api,
            &editor,
            owner.vault,
            Body::from(r#"{"title":"Must stay in trash"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = h
        .app
        .clone()
        .oneshot(request("GET", &trash, &editor, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = text(response).await;
    assert!(html.contains("Selected recoverable document"));
    assert!(html.contains(&format!("{scoped}/restore")));
    assert!(html.contains(&format!("{scoped}/delete-forever")));

    let response = h
        .app
        .clone()
        .oneshot(selected_vault_request(
            "POST",
            &format!("{api}/restore"),
            &editor,
            owner.vault,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let restored: Document = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(restored.id, document.id);
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &format!("{scoped}/delete"),
            &editor,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &format!("{scoped}/restore"),
            &editor,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], trash);

    let response = h
        .app
        .clone()
        .oneshot(selected_vault_request(
            "DELETE",
            &api,
            &editor,
            owner.vault,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = h
        .app
        .clone()
        .oneshot(selected_vault_request(
            "DELETE",
            &format!("{api}/purge"),
            &editor,
            owner.vault,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &format!("{scoped}/delete-forever"),
            &editor,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], trash);
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &format!("{scoped}/restore"),
            &editor,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        h.vault_repository
            .default_for_user(&editor.user)
            .await
            .unwrap(),
        Some(editor.vault)
    );
}

async fn assert_selected_trash_mutations_forbidden(
    h: &Harness,
    user: &SignedIn,
    vault: VaultId,
    id: uuid::Uuid,
) {
    let api = format!("/api/documents/{id}");
    for (method, path) in [
        ("DELETE", api.clone()),
        ("POST", format!("{api}/restore")),
        ("DELETE", format!("{api}/purge")),
        (
            "POST",
            format!("/vaults/{vault}/documents/{id}/delete-forever"),
        ),
    ] {
        let response = h
            .app
            .clone()
            .oneshot(selected_vault_request(
                method,
                &path,
                user,
                vault,
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
    }
}

#[tokio::test]
async fn selected_vault_trash_keeps_viewer_and_contributor_ownership_restrictions() {
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "trash-permission-owner").await;
    let contributor = sign_in(&h, "trash-permission-contributor").await;
    let viewer = sign_in(&h, "trash-permission-viewer").await;
    h.members
        .grant(
            owner.vault,
            &Principal::User(contributor.user.clone()),
            VaultRole::Contributor,
            None,
        )
        .await
        .unwrap();
    h.members
        .grant(
            owner.vault,
            &Principal::User(viewer.user.clone()),
            VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();
    let owned = h
        .state
        .document_repo
        .create(
            owner.vault,
            CreateDocument {
                title: "Contributor upload".into(),
                uploaded_by: Some(contributor.user.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let other = h
        .state
        .document_repo
        .create(
            owner.vault,
            CreateDocument {
                title: "Owner upload".into(),
                uploaded_by: Some(owner.user.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.state
        .document_repo
        .delete(owner.vault, owned.id)
        .await
        .unwrap();
    h.state
        .document_repo
        .delete(owner.vault, other.id)
        .await
        .unwrap();

    assert_selected_trash_mutations_forbidden(&h, &viewer, owner.vault, owned.id).await;
    assert_selected_trash_mutations_forbidden(&h, &contributor, owner.vault, other.id).await;

    let api = format!("/api/documents/{}", owned.id);
    let response = h
        .app
        .clone()
        .oneshot(selected_vault_request(
            "POST",
            &format!("{api}/restore"),
            &contributor,
            owner.vault,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = h
        .app
        .clone()
        .oneshot(selected_vault_request(
            "DELETE",
            &api,
            &contributor,
            owner.vault,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = h
        .app
        .clone()
        .oneshot(selected_vault_request(
            "DELETE",
            &format!("{api}/purge"),
            &contributor,
            owner.vault,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(h
        .state
        .document_repo
        .find_trashed_by_id(owner.vault, other.id)
        .await
        .unwrap()
        .unwrap()
        .purge_requested_at
        .is_none());
}

#[tokio::test]
async fn event_stream_rejects_moved_and_unknown_origins_without_exposing_context() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "captured-stream-alice").await;
    let bob = sign_in(&h, "captured-stream-bob").await;
    let moved = h
        .state
        .document_repo
        .create(
            bob.vault,
            CreateDocument {
                title: "old-origin document".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let old_origin = captured(
        &h,
        bob.vault,
        &DocumentEvent::Updated {
            document_id: moved.id,
        },
    )
    .await;
    h.state
        .document_repo
        .move_to(moved.id, bob.vault, alice.vault, None)
        .await
        .unwrap();
    let visible = h
        .state
        .document_repo
        .create(
            alice.vault,
            CreateDocument {
                title: "allowed sentinel".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut permitted = captured(
        &h,
        alice.vault,
        &DocumentEvent::Updated {
            document_id: visible.id,
        },
    )
    .await;
    std::sync::Arc::make_mut(permitted.context.as_mut().unwrap()).title =
        "internal-context-only".into();
    let removed = h
        .state
        .document_repo
        .create(
            alice.vault,
            CreateDocument {
                title: "legacy removal".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.state
        .document_repo
        .delete(alice.vault, removed.id)
        .await
        .unwrap();
    h.state
        .document_repo
        .request_purge(alice.vault, removed.id)
        .await
        .unwrap();
    h.state
        .document_repo
        .finalize_purge(alice.vault, removed.id)
        .await
        .unwrap();
    let mut legacy_deleted = h.state.outbox.replay(0, chrono::Utc::now() - chrono::Duration::days(1), 100).await.unwrap().into_iter().find(|event| matches!(event.event, Some(DocumentEvent::Deleted { document_id, .. }) if document_id == removed.id)).unwrap();
    legacy_deleted.context = None;
    legacy_deleted.envelope = None;
    let response = h
        .app
        .clone()
        .oneshot(request("GET", "/events", &alice, Body::empty()))
        .await
        .unwrap();
    let mut body = response.into_body();
    h.state.event_broadcaster.send(old_origin);
    h.state.event_broadcaster.send(OutboxEvent {
        id: 900_000,
        event: Some(DocumentEvent::Updated {
            document_id: visible.id,
        }),
        envelope: None,
        origin: EventOrigin::User,
        context: None,
        created_at: chrono::Utc::now(),
        attempts: 1,
    });
    h.state.event_broadcaster.send(legacy_deleted);
    h.state.event_broadcaster.send(permitted);

    let delivered = stream_until(&mut body, "allowed sentinel").await;

    assert!(delivered.contains(&format!("document-{}", removed.id)));
    assert!(!delivered.contains(&moved.id.to_string()));
    assert!(!delivered.contains("old-origin document"));
    assert!(!delivered.contains("900000"));
    assert!(!delivered.contains("internal-context-only"));
    assert!(!delivered.contains("\"context\""));
}

#[tokio::test]
async fn viewer_cannot_change_document_date_or_receive_edit_capability() {
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "date-owner").await;
    let viewer = sign_in(&h, "date-viewer").await;
    let doc = h
        .state
        .document_repo
        .create(
            owner.vault,
            CreateDocument {
                title: "Dated letter".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.members
        .grant(
            owner.vault,
            &Principal::User(viewer.user.clone()),
            VaultRole::Viewer,
            None,
        )
        .await
        .unwrap();
    let path = format!("/vaults/{}/api/documents/{}", owner.vault, doc.id);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &viewer, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(json["can_edit_document_date"], false);
    let response = h
        .app
        .clone()
        .oneshot(request(
            "PATCH",
            &path,
            &viewer,
            Body::from(r#"{"document_date":"2026-11-02"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        h.state
            .document_repo
            .find_by_id(owner.vault, doc.id)
            .await
            .unwrap()
            .unwrap()
            .document_date,
        None
    );
}

#[tokio::test]
async fn source_outcomes_require_owner_permission_even_for_editors() {
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "source-outcomes-owner").await;
    let editor = sign_in(&h, "source-outcomes-editor").await;
    h.members
        .grant(
            owner.vault,
            &Principal::User(editor.user.clone()),
            VaultRole::Editor,
            None,
        )
        .await
        .unwrap();
    let source = h.state.ingestion.managed.create(owner.vault, serde_json::from_value(serde_json::json!({
        "name": "Private source", "config": {"type": "folder", "path": "/isolated/nonexistent-source"}
    })).unwrap()).await.unwrap();
    let path = format!("/api/sources/{}/outcomes", source.id);
    let mut denied = request("GET", &path, &editor, Body::empty());
    denied
        .headers_mut()
        .insert("X-Folio-Vault", owner.vault.to_string().parse().unwrap());

    let response = h.app.clone().oneshot(denied).await.unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let allowed = h
        .app
        .clone()
        .oneshot(request("GET", &path, &owner, Body::empty()))
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);
}
