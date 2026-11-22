mod common;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use common::{harness_requiring_login, text, Harness};
use folio_auth::{crypto::hash_token, Session, User};
use folio_core::models::{Principal, UserId, VaultId, VaultRole};
use serde_json::{json, Value};
use tower::ServiceExt;

struct Account {
    user: UserId,
    vault: VaultId,
    cookie: String,
}

async fn account(h: &Harness, name: &str) -> Account {
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
    h.auth_store
        .create_session(Session {
            id_hash: hash_token(name),
            user_id: name.into(),
            expires_at: i64::MAX,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    let user = UserId::from(name);
    let vault = h.state.vaults.provision_user(&user).await.unwrap();
    Account {
        user,
        vault,
        cookie: format!("folio_session={name}"),
    }
}

fn request(method: &str, path: &str, cookie: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost")
        .header(header::ORIGIN, "http://localhost")
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn call(
    h: &Harness,
    method: &str,
    path: &str,
    cookie: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = h
        .app
        .clone()
        .oneshot(request(method, path, cookie, body))
        .await
        .unwrap();
    let status = response.status();
    let body = text(response).await;
    (
        status,
        if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&body).unwrap()
        },
    )
}

async fn create_field(h: &Harness, user: &Account, kind: &str) -> Value {
    let (status, field) = call(
        h,
        "POST",
        &format!("/api/vaults/{}/metadata-fields", user.vault),
        &user.cookie,
        json!({"name":" Invoice number ", "kind":kind, "required":true, "unique":true}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{field}");
    field
}

#[tokio::test]
async fn field_lifecycle_preserves_identity_and_server_attribution() {
    let h = harness_requiring_login().await;
    let owner = account(&h, "owner").await;
    let field = create_field(&h, &owner, "text").await;
    assert_eq!(field["name"], "Invoice number");
    assert_eq!(field["created_by"], "owner");
    let collection = format!("/api/vaults/{}/metadata-fields", owner.vault);
    let path = format!("{collection}/{}", field["id"].as_str().unwrap());
    let (status, updated) = call(
        &h,
        "PATCH",
        &path,
        &owner.cookie,
        json!({"name":"Reference", "kind":"text", "required":true, "unique":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["id"], field["id"]);
    assert_eq!(updated["required"], true);
    assert_eq!(updated["unique"], true);
    assert_eq!(updated["kind"], "text");
    assert_eq!(
        call(&h, "DELETE", &path, &owner.cookie, Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&h, "GET", &collection, &owner.cookie, Value::Null)
            .await
            .1,
        json!([])
    );
    let retained = call(
        &h,
        "GET",
        &format!("{collection}?include_deleted=true"),
        &owner.cookie,
        Value::Null,
    )
    .await
    .1;
    assert_eq!(retained[0]["id"], field["id"]);
    assert!(retained[0]["deleted_at"].is_string());
    let restored = call(
        &h,
        "POST",
        &format!("{path}/restore"),
        &owner.cookie,
        Value::Null,
    )
    .await;
    assert_eq!(restored.0, StatusCode::OK);
    assert_eq!(restored.1["deleted_at"], Value::Null);
}

#[tokio::test]
async fn options_remain_scoped_to_their_select_field_through_lifecycle() {
    let h = harness_requiring_login().await;
    let owner = account(&h, "owner").await;
    let field = create_field(&h, &owner, "select").await;
    let base = format!(
        "/api/vaults/{}/metadata-fields/{}",
        owner.vault,
        field["id"].as_str().unwrap()
    );
    let collection = format!("{base}/options");
    let (status, option) = call(
        &h,
        "POST",
        &collection,
        &owner.cookie,
        json!({"label":" Open ","position":4}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(option["label"], "Open");
    assert_eq!(option["created_by"], "owner");
    let path = format!("{collection}/{}", option["id"].as_str().unwrap());
    let (status, renamed) = call(
        &h,
        "PATCH",
        &path,
        &owner.cookie,
        json!({"label":"Pending", "position":4}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["id"], option["id"]);
    assert_eq!(renamed["position"], 4);
    assert_eq!(
        call(&h, "DELETE", &path, &owner.cookie, Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&h, "GET", &collection, &owner.cookie, Value::Null)
            .await
            .1,
        json!([])
    );
    assert_eq!(
        call(
            &h,
            "GET",
            &format!("{collection}?include_deleted=true"),
            &owner.cookie,
            Value::Null
        )
        .await
        .1[0]["id"],
        option["id"]
    );
    assert_eq!(
        call(
            &h,
            "POST",
            &format!("{path}/restore"),
            &owner.cookie,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    let wrong_field = format!(
        "/api/vaults/{}/metadata-fields/{}/options/{}",
        owner.vault,
        uuid::Uuid::new_v4(),
        option["id"].as_str().unwrap()
    );
    assert_eq!(
        call(&h, "GET", &wrong_field, &owner.cookie, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &h,
            "GET",
            &format!(
                "/api/vaults/{}/metadata-fields/{}/options",
                owner.vault,
                uuid::Uuid::new_v4()
            ),
            &owner.cookie,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

async fn role_contract(role: VaultRole, may_manage: bool) {
    let h = harness_requiring_login().await;
    let owner = account(&h, "owner").await;
    let member = account(&h, "member").await;
    let field = create_field(&h, &owner, "select").await;
    h.members
        .grant(
            owner.vault,
            &Principal::User(member.user.clone()),
            role,
            Some(&owner.user),
        )
        .await
        .unwrap();
    let base = format!("/api/vaults/{}/metadata-fields", owner.vault);
    let item = format!("{base}/{}", field["id"].as_str().unwrap());
    let (status, option) = call(
        &h,
        "POST",
        &format!("{item}/options"),
        &owner.cookie,
        json!({"label":"Fixture","position":2}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let option_path = format!("{item}/options/{}", option["id"].as_str().unwrap());
    assert_eq!(
        call(&h, "GET", &option_path, &member.cookie, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    let result = call(&h, "GET", &item, &member.cookie, Value::Null).await;
    assert_eq!(result.0, StatusCode::OK);
    assert_eq!(result.1["name"], "Invoice number");
    let expected = if may_manage {
        StatusCode::CREATED
    } else {
        StatusCode::FORBIDDEN
    };
    assert_eq!(
        call(
            &h,
            "POST",
            &base,
            &member.cookie,
            json!({"name":"Member field","kind":"text"})
        )
        .await
        .0,
        expected
    );
    assert_eq!(
        call(
            &h,
            "POST",
            &format!("{item}/options"),
            &member.cookie,
            json!({"label":"Open"})
        )
        .await
        .0,
        expected
    );
    let expected_update = if may_manage {
        StatusCode::OK
    } else {
        StatusCode::FORBIDDEN
    };
    assert_eq!(
        call(
            &h,
            "PATCH",
            &option_path,
            &member.cookie,
            json!({"label":"Fixture renamed","position":3})
        )
        .await
        .0,
        expected_update
    );
    let expected_delete = if may_manage {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::FORBIDDEN
    };
    assert_eq!(
        call(&h, "DELETE", &option_path, &member.cookie, Value::Null)
            .await
            .0,
        expected_delete
    );
    assert_eq!(
        call(
            &h,
            "POST",
            &format!("{option_path}/restore"),
            &member.cookie,
            Value::Null
        )
        .await
        .0,
        expected_update
    );
    assert_eq!(
        call(
            &h,
            "PATCH",
            &item,
            &member.cookie,
            json!({"name":"Renamed", "kind":"select", "required":true, "unique":true})
        )
        .await
        .0,
        expected_update
    );
    let expected_delete = if may_manage {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::FORBIDDEN
    };
    assert_eq!(
        call(&h, "DELETE", &item, &member.cookie, Value::Null)
            .await
            .0,
        expected_delete
    );
    assert_eq!(
        call(
            &h,
            "POST",
            &format!("{item}/restore"),
            &member.cookie,
            Value::Null
        )
        .await
        .0,
        expected_update
    );
    h.members
        .revoke(owner.vault, &Principal::User(member.user))
        .await
        .unwrap();
    assert_eq!(
        call(&h, "GET", &item, &member.cookie, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn viewers_read_fields_and_options_but_cannot_manage_them() {
    role_contract(VaultRole::Viewer, false).await;
}
#[tokio::test]
async fn contributors_cannot_manage_fields_or_options() {
    role_contract(VaultRole::Contributor, false).await;
}
#[tokio::test]
async fn editors_manage_fields_and_options() {
    role_contract(VaultRole::Editor, true).await;
}

#[tokio::test]
async fn hidden_vaults_and_cross_vault_field_ids_are_not_disclosed() {
    let h = harness_requiring_login().await;
    let owner = account(&h, "owner").await;
    let other = account(&h, "other").await;
    let field = create_field(&h, &owner, "text").await;
    let hidden = format!(
        "/api/vaults/{}/metadata-fields/{}",
        owner.vault,
        field["id"].as_str().unwrap()
    );
    assert_eq!(
        call(&h, "GET", &hidden, &other.cookie, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    let crossed = format!(
        "/api/vaults/{}/metadata-fields/{}",
        other.vault,
        field["id"].as_str().unwrap()
    );
    assert_eq!(
        call(&h, "GET", &crossed, &other.cookie, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &h,
            "PATCH",
            &crossed,
            &other.cookie,
            json!({"name":"Stolen", "kind":"text"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn validation_conflicts_pagination_and_csrf_use_json_api_statuses() {
    let h = harness_requiring_login().await;
    let owner = account(&h, "owner").await;
    let field = create_field(&h, &owner, "text").await;
    let base = format!("/api/vaults/{}/metadata-fields", owner.vault);
    assert_eq!(
        call(
            &h,
            "POST",
            &base,
            &owner.cookie,
            json!({"name":"invoice NUMBER","kind":"text"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &h,
            "POST",
            &base,
            &owner.cookie,
            json!({"name":" ","kind":"text"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &h,
            "POST",
            &base,
            &owner.cookie,
            json!({"name":"Forged","kind":"text","created_by":"other"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &h,
            "GET",
            &format!("{base}?limit=101"),
            &owner.cookie,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &h,
            "GET",
            &format!("{base}?limit=0"),
            &owner.cookie,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &h,
            "GET",
            &format!("{base}?after={}", field["id"].as_str().unwrap()),
            &owner.cookie,
            Value::Null
        )
        .await
        .1,
        json!([])
    );
    let mut cross_site = request(
        "POST",
        &base,
        &owner.cookie,
        json!({"name":"Cross site","kind":"text"}),
    );
    cross_site
        .headers_mut()
        .insert(header::ORIGIN, "https://other.invalid".parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(cross_site).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&h, "GET", &base, "", Value::Null).await.0,
        StatusCode::UNAUTHORIZED
    );
    let item = format!("{base}/{}", field["id"].as_str().unwrap());
    assert_eq!(
        call(&h, "DELETE", &item, &owner.cookie, Value::Null)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let replacement = create_field(&h, &owner, "text").await;
    assert_ne!(replacement["id"], field["id"]);
    assert_eq!(
        call(
            &h,
            "POST",
            &format!("{item}/restore"),
            &owner.cookie,
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn bearer_scope_and_live_vault_roles_both_apply_to_metadata() {
    let h = harness_requiring_login().await;
    let owner = account(&h, "owner").await;
    let member = account(&h, "member").await;
    h.members
        .grant(
            owner.vault,
            &Principal::User(member.user.clone()),
            VaultRole::Viewer,
            Some(&owner.user),
        )
        .await
        .unwrap();
    let (_, token) = call(&h, "POST", "/api/auth/tokens", &member.cookie, json!({"name":"Metadata reader","audience":"api","scopes":["documents:read", "documents:write"],"expires_in_days":30})).await;
    let base = format!("/api/vaults/{}/metadata-fields", owner.vault);
    let bearer = format!("Bearer {}", token["token"].as_str().unwrap());
    let mut read = request("GET", &base, "", Value::Null);
    read.headers_mut()
        .insert(header::AUTHORIZATION, bearer.parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(read).await.unwrap().status(),
        StatusCode::OK
    );
    let mut write = request(
        "POST",
        &base,
        &owner.cookie,
        json!({"name":"Forbidden","kind":"text"}),
    );
    write
        .headers_mut()
        .insert(header::AUTHORIZATION, bearer.parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(write).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    h.members
        .revoke(owner.vault, &Principal::User(member.user))
        .await
        .unwrap();
    let mut read = request("GET", &base, "", Value::Null);
    read.headers_mut()
        .insert(header::AUTHORIZATION, bearer.parse().unwrap());
    assert_eq!(
        h.app.clone().oneshot(read).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn personal_tokens_require_independent_read_and_write_scopes_for_metadata() {
    let h = harness_requiring_login().await;
    let owner = account(&h, "owner").await;
    let (_, read_token) = call(
        &h,
        "POST",
        "/api/auth/tokens",
        &owner.cookie,
        json!({"name":"Reader","audience":"api","scopes":["documents:read"],"expires_in_days":30}),
    )
    .await;
    let (_, write_token) = call(
        &h,
        "POST",
        "/api/auth/tokens",
        &owner.cookie,
        json!({"name":"Writer","audience":"api","scopes":["documents:write"],"expires_in_days":30}),
    )
    .await;
    let base = format!("/api/vaults/{}/metadata-fields", owner.vault);
    let mut forbidden_write = request("POST", &base, "", json!({"name":"Forbidden","kind":"text"}));
    forbidden_write.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {}", read_token["token"].as_str().unwrap())
            .parse()
            .unwrap(),
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(forbidden_write)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let mut forbidden_read = request("GET", &base, "", Value::Null);
    forbidden_read.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {}", write_token["token"].as_str().unwrap())
            .parse()
            .unwrap(),
    );
    assert_eq!(
        h.app
            .clone()
            .oneshot(forbidden_read)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let mut allowed_write = request("POST", &base, "", json!({"name":"Allowed","kind":"text"}));
    allowed_write.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {}", write_token["token"].as_str().unwrap())
            .parse()
            .unwrap(),
    );
    let response = h.app.clone().oneshot(allowed_write).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let created: Value = serde_json::from_str(&text(response).await).unwrap();
    assert_eq!(created["created_by"], "owner");
}

#[tokio::test]
async fn document_patch_preserves_exact_values_and_rejects_stale_or_invalid_changes() {
    let h = harness_requiring_login().await;
    let owner = account(&h, "metadata-owner").await;
    let document = metadata_document(&h, &owner, &owner).await;
    let path = format!("/api/vaults/{}/documents/{document}/metadata", owner.vault);
    let decimal = create_field(&h, &owner, "decimal").await;
    let (status, flag) = call(
        &h,
        "POST",
        &format!("/api/vaults/{}/metadata-fields", owner.vault),
        &owner.cookie,
        json!({"name":"Paid","kind":"boolean"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let before = call(&h, "GET", &path, &owner.cookie, Value::Null).await;
    assert_eq!(before.0, StatusCode::OK);
    assert_eq!(before.1["revision"], Value::Null);
    assert_eq!(before.1["values"], json!([]));

    let patch = json!({"revision":null,"changes":[
        {"field_id":decimal["id"],"value":{"kind":"decimal","value":"9007199254740993.010"}},
        {"field_id":flag["id"],"value":{"kind":"boolean","value":false}}
    ]});
    let (status, applied) = call(&h, "PATCH", &path, &owner.cookie, patch.clone()).await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    let values = applied["values"].as_array().unwrap();
    assert!(values
        .iter()
        .any(|item| item["value"] == json!({"kind":"boolean","value":false})));
    assert!(values
        .iter()
        .any(|item| item["value"] == json!({"kind":"decimal","value":"9007199254740993.01"})));
    assert!(values
        .iter()
        .all(|item| item["updated_by"] == "metadata-owner"));
    assert!(applied["revision"].is_string());

    assert_eq!(
        call(&h, "PATCH", &path, &owner.cookie, patch).await.0,
        StatusCode::CONFLICT
    );
    let invalid = json!({"revision":applied["revision"],"changes":[
        {"field_id":flag["id"],"value":null},
        {"field_id":decimal["id"],"value":{"kind":"decimal","value":"1e3"}}
    ]});
    assert_eq!(
        call(&h, "PATCH", &path, &owner.cookie, invalid).await.0,
        StatusCode::BAD_REQUEST
    );
    let after = call(&h, "GET", &path, &owner.cookie, Value::Null).await;
    assert_eq!(after.0, StatusCode::OK);
    assert_eq!(after.1, applied);
}

async fn metadata_document(h: &Harness, vault: &Account, uploader: &Account) -> uuid::Uuid {
    use folio_core::models::CreateDocument;
    h.state
        .document_repo
        .create(
            vault.vault,
            CreateDocument {
                title: "Metadata fixture".into(),
                uploaded_by: Some(uploader.user.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn document_patch_checks_upload_ownership_and_hides_out_of_vault_link_targets() {
    let h = harness_requiring_login().await;
    let owner = account(&h, "metadata-owner").await;
    let contributor = account(&h, "metadata-contributor").await;
    h.members
        .grant(
            owner.vault,
            &Principal::User(contributor.user.clone()),
            VaultRole::Contributor,
            Some(&owner.user),
        )
        .await
        .unwrap();
    let attached = metadata_document(&h, &owner, &contributor).await;
    let other = metadata_document(&h, &owner, &owner).await;
    let hidden = metadata_document(&h, &contributor, &contributor).await;
    let path = |document| format!("/api/vaults/{}/documents/{document}/metadata", owner.vault);
    let field = create_field(&h, &owner, "document_link").await;
    let body = |target| {
        json!({"revision":null,"changes":[
            {"field_id":field["id"],"value":{"kind":"document_link","value":target}}
        ]})
    };
    assert_eq!(
        call(&h, "GET", &path(other), &contributor.cookie, Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &h,
            "PATCH",
            &path(other),
            &contributor.cookie,
            body(attached)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let inaccessible = call(
        &h,
        "PATCH",
        &path(attached),
        &contributor.cookie,
        body(hidden),
    )
    .await;
    let missing = call(
        &h,
        "PATCH",
        &path(attached),
        &contributor.cookie,
        body(uuid::Uuid::new_v4()),
    )
    .await;
    assert_eq!(inaccessible.0, StatusCode::NOT_FOUND);
    assert_eq!(inaccessible, missing);
    assert_eq!(
        call(
            &h,
            "PATCH",
            &path(attached),
            &contributor.cookie,
            body(other)
        )
        .await
        .0,
        StatusCode::OK
    );
    h.state
        .document_repo
        .delete(owner.vault, other)
        .await
        .unwrap();
    let (status, retained) =
        call(&h, "GET", &path(attached), &contributor.cookie, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        retained["values"][0]["value"],
        json!({"kind":"document_link","value":other})
    );
    assert_eq!(
        retained["values"][0]["link_target"],
        json!({"title":"Metadata fixture","trashed":true})
    );
    assert_eq!(
        call(&h, "GET", &path(hidden), &contributor.cookie, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn oversized_document_transition_rolls_back_without_an_event() {
    let h = harness_requiring_login().await;
    let owner = account(&h, "metadata-owner").await;
    let document = metadata_document(&h, &owner, &owner).await;
    let collection = format!("/api/vaults/{}/metadata-fields", owner.vault);
    let path = format!("/api/vaults/{}/documents/{document}/metadata", owner.vault);
    let mut changes = Vec::new();
    for index in 0..3 {
        let (status, field) = call(
            &h,
            "POST",
            &collection,
            &owner.cookie,
            json!({"name":format!("Notes {index}"),"kind":"long_text"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{field}");
        changes.push(json!({
            "field_id":field["id"],
            "value":{"kind":"long_text","value":"\u{0001}".repeat(65_536)}
        }));
    }
    let (status, body) = call(
        &h,
        "PATCH",
        &path,
        &owner.cookie,
        json!({"revision":null,"changes":changes}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, metadata) = call(&h, "GET", &path, &owner.cookie, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(metadata["revision"], Value::Null);
    assert_eq!(metadata["values"], json!([]));
    let events = h
        .state
        .outbox
        .replay(0, chrono::DateTime::UNIX_EPOCH, 50)
        .await
        .unwrap();
    assert!(!events
        .iter()
        .any(|row| row.envelope.as_ref().is_some_and(
            |event| event.event_type() == folio_core::domain_event::DOCUMENT_METADATA_CHANGED
        )));
}

#[tokio::test]
async fn exact_search_reads_tagged_values_only_in_the_selected_vault() {
    use folio_core::models::{CreateDocument, MetadataChange, MetadataValue};

    let h = harness_requiring_login().await;
    let owner = account(&h, "search-owner").await;
    let outsider = account(&h, "search-outsider").await;
    let field = create_field(&h, &owner, "decimal").await;
    let document = h
        .state
        .document_repo
        .create(
            owner.vault,
            CreateDocument {
                title: "Exact amount".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.state
        .metadata_values
        .patch(
            owner.vault,
            document.id,
            None,
            vec![MetadataChange {
                field_id: uuid::Uuid::parse_str(field["id"].as_str().unwrap()).unwrap(),
                value: Some(MetadataValue::Decimal("9007199254740993.01".into())),
            }],
        )
        .await
        .unwrap();
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("field_id", field["id"].as_str().unwrap())
        .append_pair(
            "value",
            r#"{"kind":"decimal","value":"9007199254740993.010"}"#,
        )
        .finish();
    let path = format!("/api/vaults/{}/metadata-search?{encoded}", owner.vault);
    let (status, page) = call(&h, "GET", &path, &owner.cookie, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        page,
        json!({"documents":[{"id":document.id,"title":"Exact amount"}],"next_cursor":null})
    );
    assert_eq!(
        call(&h, "GET", &path, &outsider.cookie, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &h,
            "GET",
            &format!("{path}&cursor=broken"),
            &owner.cookie,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
