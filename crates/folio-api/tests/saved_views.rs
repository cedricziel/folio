mod common;
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::{harness_requiring_login, Harness};
use folio_auth::{crypto::hash_token, Session, User};
use folio_core::models::{UserId, VaultId};
use tower::ServiceExt;
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

#[tokio::test]
async fn rename_preserves_query_and_audit_identity_until_deleted() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let collection = format!("/api/vaults/{}/saved-views", alice.vault);
    let response = h.app.clone().oneshot(request(
        "POST", &collection, &alice,
        Body::from(r#"{"name":"Invoices","query":{"version":1,"filters":{"created_after":"2024-01-01T00:00:00.123456789Z"}},"presentation":"grid"}"#),
    )).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let original: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    let path = format!("{}/{}", collection, original["id"].as_str().unwrap());
    let response = h
        .app
        .clone()
        .oneshot(request(
            "PATCH",
            &path,
            &alice,
            Body::from(r#"{"name":"Tax invoices"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let renamed: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(renamed["name"], "Tax invoices");
    assert_eq!(renamed["query"], original["query"]);
    assert_eq!(renamed["presentation"], "grid");
    assert_eq!(renamed["owner_id"], "alice");
    assert_eq!(renamed["created_by"], "alice");
    assert_eq!(renamed["updated_by"], "alice");
    assert_eq!(renamed["created_at"], original["created_at"]);
    for body in [r#"{"query":null}"#, r#"{"owner_id":"someone-else"}"#] {
        let response = h
            .app
            .clone()
            .oneshot(request("PATCH", &path, &alice, Body::from(body)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let retained: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(retained, renamed);
    let response = h
        .app
        .clone()
        .oneshot(request("DELETE", &path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn private_saved_view_is_hidden_from_another_member_and_vault() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let bob = sign_in(&h, "bob").await;
    let path = format!("/api/vaults/{}/saved-views", alice.vault);
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &path,
            &alice,
            Body::from(r#"{"name":"Invoices","query":{"version":1}}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(body["owner_id"], "alice");
    assert_eq!(body["created_by"], "alice");
    let id = body["id"].as_str().unwrap();
    let response = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/vaults/{}/saved-views/{id}", bob.vault),
            &bob,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &format!("{path}/{id}"), &bob, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn viewer_can_save_privately_but_cannot_publish() {
    use folio_core::models::{Principal, VaultRole};
    let h = harness_requiring_login().await;
    let owner = sign_in(&h, "owner").await;
    let viewer = sign_in(&h, "viewer").await;
    h.members
        .grant(
            owner.vault,
            &Principal::User(viewer.user.clone()),
            VaultRole::Viewer,
            Some(&owner.user),
        )
        .await
        .unwrap();
    let path = format!("/api/vaults/{}/saved-views", owner.vault);
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &path,
            &viewer,
            Body::from(r#"{"name":"Private","query":{"version":1}}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &path,
            &viewer,
            Body::from(r#"{"name":"Shared","visibility":"shared","query":{"version":1}}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn cross_vault_filter_is_rejected_without_saving() {
    use folio_core::models::CreateTag;
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let bob = sign_in(&h, "bob").await;
    let tag = h
        .state
        .tag_repo
        .create(
            bob.vault,
            CreateTag {
                name: "Secret".into(),
                color: None,
            },
        )
        .await
        .unwrap();
    let path = format!("/api/vaults/{}/saved-views", alice.vault);
    let input =
        serde_json::json!({"name":"Invalid", "query":{"version":1,"filters":{"tag_ids":[tag.id]}}});
    let response = h
        .app
        .clone()
        .oneshot(request(
            "POST",
            &path,
            &alice,
            Body::from(input.to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(common::text(response).await, "[]");
}

#[tokio::test]
async fn cross_origin_saved_view_form_is_rejected() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let mut req = request(
        "POST",
        &format!("/vaults/{}/saved-views", alice.vault),
        &alice,
        Body::from("name=Bad&visibility=private&presentation=list"),
    );
    req.headers_mut()
        .insert(header::ORIGIN, "https://untrusted.invalid".parse().unwrap());
    req.headers_mut().insert(
        header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let response = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unavailable_filter_remains_explicit_and_deletable_in_web_editor() {
    use folio_core::models::{
        DocumentFilter, SavedViewInput, SavedViewQuery, VaultRole, ViewPresentation, ViewVisibility,
    };
    use folio_core::ports::SavedViewAccess;
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let missing_tag = uuid::Uuid::new_v4();
    let view = h
        .state
        .saved_views
        .create(
            alice.vault,
            &SavedViewAccess {
                user: alice.user.clone(),
                role: VaultRole::Owner,
            },
            SavedViewInput {
                name: "Retained filter".into(),
                visibility: ViewVisibility::Private,
                presentation: ViewPresentation::List,
                query: SavedViewQuery {
                    filters: DocumentFilter {
                        tag_ids: Some(vec![missing_tag]),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();
    let response = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/vaults/{}/saved-views/{}", alice.vault, view.id),
            &alice,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = common::text(response).await;
    assert!(body.contains("A saved-view tag is unavailable"));
    assert!(body.contains(&format!("value=\"{missing_tag}\" checked")));
    assert!(body.contains("Delete saved view"));
}

#[tokio::test]
async fn web_editor_preserves_fractional_date_boundaries() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let path = format!("/api/vaults/{}/saved-views", alice.vault);
    let response = h.app.clone().oneshot(request(
        "POST", &path, &alice,
        Body::from(r#"{"name":"Precise dates","query":{"version":1,"filters":{"created_after":"2026-01-01T12:00:00.123456789Z"}}}"#),
    )).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let view: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    let id = view["id"].as_str().unwrap();
    let web_path = format!("/vaults/{}/saved-views/{id}", alice.vault);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &web_path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(common::text(response)
        .await
        .contains("value=\"2026-01-01T12:00:00.123\""));
    let mut update = request("POST", &web_path, &alice, Body::from("name=Renamed&visibility=private&presentation=list&created_after=2026-01-01T12%3A00%3A00.123&created_after_original=2026-01-01T12%3A00%3A00.123456789Z"));
    update.headers_mut().insert(
        header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let response = h.app.clone().oneshot(update).await.unwrap();
    assert!(response.status().is_redirection());
    let response = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("{path}/{id}"),
            &alice,
            Body::empty(),
        ))
        .await
        .unwrap();
    let updated: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(
        updated["query"]["filters"]["created_after"],
        view["query"]["filters"]["created_after"]
    );
}

#[tokio::test]
async fn web_editor_keeps_review_and_document_date_filters() {
    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let new_path = format!(
        "/vaults/{}/saved-views/new?reviewed=false&document_date_from=2026-01-01&document_date_to=2026-01-31",
        alice.vault
    );
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &new_path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = common::text(response).await;
    assert!(html.contains("<option value=\"false\" selected>Not reviewed</option>"));
    assert!(html.contains("name=\"document_date_from\" value=\"2026-01-01\""));
    assert!(html.contains("name=\"document_date_to\" value=\"2026-01-31\""));

    let mut create = request(
        "POST",
        &format!("/vaults/{}/saved-views", alice.vault),
        &alice,
        Body::from("name=Unreviewed&visibility=private&presentation=list&reviewed=false&document_date_from=2026-01-01&document_date_to=2026-01-31"),
    );
    create.headers_mut().insert(
        header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let response = h.app.clone().oneshot(create).await.unwrap();
    assert!(response.status().is_redirection());
    let response = h
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/api/vaults/{}/saved-views", alice.vault),
            &alice,
            Body::empty(),
        ))
        .await
        .unwrap();
    let views: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    let filters = &views[0]["query"]["filters"];
    assert_eq!(filters["reviewed"], false);
    assert_eq!(filters["document_date_from"], "2026-01-01");
    assert_eq!(filters["document_date_to"], "2026-01-31");
}

#[tokio::test]
async fn saving_inbox_keeps_reviewed_documents_until_they_are_filed() {
    use folio_core::models::{CreateDocument, ProcessingStage, ProcessingStatus};

    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let repo = &h.state.document_repo;
    for (title, status, reviewed) in [
        ("Inbox fixture processing", ProcessingStatus::Running, true),
        ("Inbox fixture failed", ProcessingStatus::Failed, true),
        ("Inbox fixture unreviewed", ProcessingStatus::Done, false),
        ("Inbox fixture filed", ProcessingStatus::Done, true),
    ] {
        let doc = repo
            .create(
                alice.vault,
                CreateDocument {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        repo.set_stage_status(
            alice.vault,
            doc.id,
            ProcessingStage::Ocr,
            ProcessingStatus::Done,
        )
        .await
        .unwrap();
        repo.set_stage_status(alice.vault, doc.id, ProcessingStage::Embed, status)
            .await
            .unwrap();
        repo.set_reviewed(alice.vault, doc.id, reviewed)
            .await
            .unwrap();
    }

    let inbox = format!("/vaults/{}/", alice.vault);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &inbox, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = common::text(response).await;
    let new_path = html
        .split("href=\"")
        .filter_map(|part| part.split('"').next())
        .find(|url| url.contains("/saved-views/new?inbox_only=true"))
        .expect("save link carries the Inbox scope");
    let response = h
        .app
        .clone()
        .oneshot(request("GET", new_path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(common::text(response)
        .await
        .contains("name=\"inbox_only\" value=\"true\""));

    let collection = format!("/vaults/{}/saved-views", alice.vault);
    let mut create = request(
        "POST",
        &collection,
        &alice,
        Body::from("name=Morning&visibility=private&presentation=list&inbox_only=true"),
    );
    create.headers_mut().insert(
        header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let response = h.app.clone().oneshot(create).await.unwrap();
    assert!(response.status().is_redirection());
    let saved = response.headers()[header::LOCATION].to_str().unwrap();
    let response = h
        .app
        .clone()
        .oneshot(request("GET", saved, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = common::text(response).await;
    assert!(html.contains("Inbox fixture processing"));
    assert!(html.contains("Inbox fixture failed"));
    assert!(html.contains("Inbox fixture unreviewed"));
    assert!(!html.contains("Inbox fixture filed"));

    let collection = format!("/api{collection}");
    let legacy_list = request("GET", &collection, &alice, Body::empty());
    let response = h.app.clone().oneshot(legacy_list).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(common::text(response).await, "[]");

    let mut compatible_list = request("GET", &collection, &alice, Body::empty());
    compatible_list
        .headers_mut()
        .insert("x-folio-saved-view-query-version", "2".parse().unwrap());
    let response = h.app.clone().oneshot(compatible_list).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let views: serde_json::Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(views[0]["query"]["version"], 2);
    assert_eq!(views[0]["query"]["filters"]["inbox_only"], true);
}

#[tokio::test]
async fn typed_metadata_view_restores_exact_results_and_rejects_an_unavailable_field() {
    use folio_core::models::{
        CreateDocument, MetadataChange, MetadataFieldInput, MetadataKind, MetadataValue,
    };
    use serde_json::{json, Value};

    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let field = h
        .state
        .metadata_fields
        .create(
            alice.vault,
            MetadataFieldInput {
                name: "Paid".into(),
                kind: MetadataKind::Boolean,
                required: false,
                unique: false,
            },
        )
        .await
        .unwrap();
    let mut unpaid_id = None;
    for (title, paid) in [("Outstanding invoice", false), ("Settled invoice", true)] {
        let document = h
            .state
            .document_repo
            .create(
                alice.vault,
                CreateDocument {
                    title: title.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        h.state
            .metadata_values
            .patch(
                alice.vault,
                document.id,
                None,
                vec![MetadataChange {
                    field_id: field.id,
                    value: Some(MetadataValue::Boolean(paid)),
                }],
            )
            .await
            .unwrap();
        if !paid {
            unpaid_id = Some(document.id);
        }
    }
    let vault = format!("/vaults/{}", alice.vault);
    let new = format!(
        "{vault}/saved-views/new?metadata_document_id={}&metadata_field_id={}",
        unpaid_id.unwrap(),
        field.id
    );
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &new, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = common::text(response).await;
    assert!(html.contains("Paid = No"));
    assert!(html.contains("Outstanding invoice"));
    assert!(!html.contains("Settled invoice"));

    let exact = json!({
        "field_id": field.id,
        "value": {"kind": "boolean", "value": false}
    });
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("name", "Outstanding")
        .append_pair("visibility", "private")
        .append_pair("presentation", "list")
        .append_pair("metadata_exact", &exact.to_string())
        .finish();
    let mut create = request(
        "POST",
        &format!("{vault}/saved-views"),
        &alice,
        Body::from(form),
    );
    create.headers_mut().insert(
        header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let response = h.app.clone().oneshot(create).await.unwrap();
    assert!(response.status().is_redirection());
    let saved = response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned();

    let collection = format!("/api{vault}/saved-views");
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &collection, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(common::text(response).await, "[]");

    let path = format!("/api{saved}");
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &path, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let current_client = || {
        let mut req = request("GET", &path, &alice, Body::empty());
        req.headers_mut()
            .insert("x-folio-saved-view-query-version", "3".parse().unwrap());
        req
    };
    let response = h.app.clone().oneshot(current_client()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value = serde_json::from_str(&common::text(response).await).unwrap();
    assert_eq!(payload["query"]["version"], 3);
    assert_eq!(payload["query"]["filters"]["metadata_exact"], exact);

    let mut browse = request(
        "POST",
        "/api/documents/browse",
        &alice,
        Body::from(json!({"query": payload["query"]}).to_string()),
    );
    browse
        .headers_mut()
        .insert("x-folio-vault", alice.vault.to_string().parse().unwrap());
    let response = h.app.clone().oneshot(browse).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value = serde_json::from_str(&common::text(response).await).unwrap();
    let documents = payload["documents"].as_array().unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0]["title"], "Outstanding invoice");

    h.state
        .metadata_fields
        .set_deleted(alice.vault, field.id, true)
        .await
        .unwrap();
    let response = h.app.clone().oneshot(current_client()).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = h
        .app
        .clone()
        .oneshot(request("GET", &saved, &alice, Body::empty()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = common::text(response).await;
    assert!(html.contains("The custom field is unavailable"));
    assert!(!html.contains("Outstanding invoice"));
    assert!(!html.contains("Settled invoice"));
}

#[tokio::test]
async fn custom_field_panel_offers_saved_views_only_for_available_references() {
    use folio_core::models::{
        CreateDocument, MetadataChange, MetadataFieldInput, MetadataKind, MetadataOptionInput,
        MetadataValue,
    };

    let h = harness_requiring_login().await;
    let alice = sign_in(&h, "alice").await;
    let document = h
        .state
        .document_repo
        .create(
            alice.vault,
            CreateDocument {
                title: "Source".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let target = h
        .state
        .document_repo
        .create(
            alice.vault,
            CreateDocument {
                title: "Reference target".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let field = |name: &str, kind| MetadataFieldInput {
        name: name.into(),
        kind,
        required: false,
        unique: false,
    };
    let select = h
        .state
        .metadata_fields
        .create(alice.vault, field("Phase", MetadataKind::Select))
        .await
        .unwrap();
    let link = h
        .state
        .metadata_fields
        .create(alice.vault, field("Reference", MetadataKind::DocumentLink))
        .await
        .unwrap();
    let option = h
        .state
        .metadata_options
        .create(
            alice.vault,
            select.id,
            MetadataOptionInput {
                label: "Pending".into(),
                position: 0,
            },
        )
        .await
        .unwrap();
    h.state
        .metadata_values
        .patch(
            alice.vault,
            document.id,
            None,
            vec![
                MetadataChange {
                    field_id: select.id,
                    value: Some(MetadataValue::Select(option.id)),
                },
                MetadataChange {
                    field_id: link.id,
                    value: Some(MetadataValue::DocumentLink(target.id)),
                },
            ],
        )
        .await
        .unwrap();

    let panel = format!(
        "/vaults/{}/documents/{}/custom-fields",
        alice.vault, document.id
    );
    let get_panel = || {
        h.app
            .clone()
            .oneshot(request("GET", &panel, &alice, Body::empty()))
    };
    let response = get_panel().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        common::text(response)
            .await
            .matches("Save matching view")
            .count(),
        2
    );

    h.state
        .metadata_options
        .set_deleted(alice.vault, select.id, option.id, true)
        .await
        .unwrap();
    h.state
        .document_repo
        .delete(alice.vault, target.id)
        .await
        .unwrap();
    let response = get_panel().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = common::text(response).await;
    assert!(html.contains("Pending (retired)"));
    assert!(!html.contains("Save matching view"));

    let mut browse = request(
        "POST",
        "/api/documents/browse",
        &alice,
        Body::from(
            serde_json::json!({
                "query": {
                    "version": 3,
                    "filters": {
                        "metadata_exact": {
                            "field_id": link.id,
                            "value": {"kind": "document_link", "value": target.id}
                        }
                    }
                }
            })
            .to_string(),
        ),
    );
    browse
        .headers_mut()
        .insert("x-folio-vault", alice.vault.to_string().parse().unwrap());
    let response = h.app.clone().oneshot(browse).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(common::text(response)
        .await
        .contains("The custom field linked document is unavailable"));
}
