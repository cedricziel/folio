//! Ingestion source API and pages, driven through the real router.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use folio_core::models::{FolderConfig, IngestionSourceDef, SourceConfig, SourceOrigin};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use common::{harness_with_sources, Harness};

async fn test_app(configured: Vec<IngestionSourceDef>) -> Harness {
    harness_with_sources(configured).await
}

async fn send(app: &Harness, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(json) => {
            builder = builder.header("content-type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn json_request(
    app: &Harness,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, text) = send(app, method, uri, body).await;
    let value = if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    };
    (status, value)
}

fn configured_folder(path: &str) -> IngestionSourceDef {
    IngestionSourceDef {
        lifecycle: None,
        vault_id: None,
        id: Uuid::new_v4(),
        name: "scans".into(),
        enabled: true,
        origin: SourceOrigin::Config,
        config: SourceConfig::Folder(
            serde_json::from_value::<FolderConfig>(json!({ "path": path })).unwrap(),
        ),
    }
}

fn mail_body(password: &str) -> Value {
    json!({
        "name": "Work mail",
        "config": {
            "type": "email",
            "host": "imap.example.test",
            "username": "me",
            "password": password,
        }
    })
}

#[tokio::test]
async fn passwords_are_masked_and_a_masked_update_keeps_the_stored_one() {
    let app = test_app(Vec::new()).await;

    let (status, created) =
        json_request(&app, "POST", "/api/sources", Some(mail_body("hunter2"))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["vault_id"], app.vault.to_string());
    assert_eq!(created["config"]["password"], "********");
    assert_eq!(created["config"]["port"], 993);
    let id = created["id"].as_str().unwrap().to_string();

    let (_, listed) = json_request(&app, "GET", "/api/sources", None).await;
    assert!(!listed.to_string().contains("hunter2"));

    let (status, unchanged) = json_request(
        &app,
        "PUT",
        &format!("/api/sources/{id}"),
        Some(mail_body("********")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unchanged["updated_at"], created["updated_at"]);
    let stored = app
        .state
        .ingestion
        .managed
        .find(app.vault, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let SourceConfig::Email(email) = stored.config else {
        panic!("email")
    };
    assert_eq!(email.password.as_deref(), Some("hunter2"));

    let (status, _) = json_request(&app, "DELETE", &format!("/api/sources/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn password_files_are_refused_for_managed_sources() {
    let app = test_app(Vec::new()).await;
    let mut body = mail_body("x");
    body["config"]["password_file"] = json!("/etc/passwd");
    let (status, _) = json_request(&app, "POST", "/api/sources", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn configured_sources_are_listed_read_only_and_can_be_tested() {
    let missing = configured_folder("/definitely/not/here");
    let existing = std::env::temp_dir().join(format!("folio-sources-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&existing).unwrap();
    let mut ok = configured_folder(existing.to_str().unwrap());
    ok.name = "tmp".into();
    let (missing_id, ok_id) = (missing.id, ok.id);
    let app = test_app(vec![missing, ok]).await;

    let (_, listed) = json_request(&app, "GET", "/api/sources", None).await;
    assert_eq!(listed[0]["origin"], "config");
    assert_eq!(listed[0]["vault_id"], app.vault.to_string());

    let (status, _) =
        json_request(&app, "DELETE", &format!("/api/sources/{missing_id}"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, bad) = json_request(
        &app,
        "POST",
        &format!("/api/sources/{missing_id}/test"),
        None,
    )
    .await;
    assert_eq!(bad["ok"], false);
    let (_, good) = json_request(&app, "POST", &format!("/api/sources/{ok_id}/test"), None).await;
    assert_eq!(good["ok"], true);
    std::fs::remove_dir_all(existing).unwrap();
}

#[tokio::test]
async fn ingest_workflows_take_only_skip_import() {
    let app = test_app(Vec::new()).await;
    let rule = json!({
        "name": "Ignore newsletters",
        "trigger": "ingest.received",
        "conditions": { "source": { "kind": "email", "email_from": { "contains": "newsletter" } } },
        "actions": [{ "type": "skip_import" }]
    });
    let (status, created) = json_request(&app, "POST", "/api/workflows", Some(rule.clone())).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        created["conditions"]["source"]["email_from"]["contains"],
        "newsletter"
    );

    let mut wrong_trigger = rule.clone();
    wrong_trigger["trigger"] = json!("document.uploaded");
    let (status, _) = json_request(&app, "POST", "/api/workflows", Some(wrong_trigger)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut with_webhook = rule.clone();
    with_webhook["actions"] = json!([{ "type": "webhook", "url": "http://localhost/x" }]);
    let (status, _) = json_request(&app, "POST", "/api/workflows", Some(with_webhook)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut bad_regex = rule;
    bad_regex["conditions"]["source"]["email_from"] = json!({ "regex": "(" });
    let (status, _) = json_request(&app, "POST", "/api/workflows", Some(bad_regex)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_sources_pages_render_and_save() {
    let app = test_app(vec![configured_folder("/consume")]).await;

    let (status, page) = send(&app, "GET", "/settings/sources", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("scans") && page.contains("/consume"));

    for uri in [
        "/settings/sources/new?kind=folder",
        "/settings/sources/new?kind=email",
    ] {
        assert_eq!(send(&app, "GET", uri, None).await.0, StatusCode::OK);
    }

    let response = app
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/settings/sources")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "kind=email&name=Mail&enabled=on&host=imap.test&port=993&security=tls\
&username=me&password=pw&mailboxes=INBOX&poll_interval_secs=300&criteria=UNSEEN\
&allowed_content_types=application%2Fpdf&mail_body=none&mail_after_import=mark_read",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection(), "{}", response.status());

    let (_, listed) = json_request(&app, "GET", "/api/sources", None).await;
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert_eq!(listed[1]["config"]["password"], "********");
    assert!(listed[0]["created_at"].is_null());
    assert!(listed[0]["updated_at"].is_null());
    assert!(listed[0]["created_by"].is_null());
}

#[tokio::test]
async fn managed_sources_can_be_trashed_and_restored_from_the_existing_page() {
    let h = test_app(Vec::new()).await;
    let (status, body) = json_request(
        &h,
        "POST",
        "/api/sources",
        Some(json!({
            "name": "Lifecycle folder", "config": {"type": "folder", "path": "/unused/source"},
            "created_by": "forged", "updated_by": "forged"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap();
    assert!(body["created_at"].is_string());
    assert!(body["created_by"].is_null());
    let (status, _) = send(&h, "POST", &format!("/settings/sources/{id}/delete"), None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (_, active) = json_request(&h, "GET", "/api/sources", None).await;
    assert!(active.as_array().unwrap().is_empty());
    let (status, page) = send(&h, "GET", "/settings/sources?deleted=true", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Lifecycle folder"));
    assert!(page.contains(&format!("/{id}/restore")));
    let (status, _) = send(&h, "POST", &format!("/settings/sources/{id}/restore"), None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (_, restored) = json_request(&h, "GET", &format!("/api/sources/{id}"), None).await;
    assert_eq!(restored["id"], body["id"]);
    assert_eq!(restored["created_at"], body["created_at"]);
    assert!(restored["deleted_at"].is_null());
}

async fn source_session(h: &Harness, name: &str, role: folio_core::models::VaultRole) -> String {
    h.auth_store
        .create_user(folio_auth::User {
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
    let user = folio_core::models::UserId::from(name);
    h.members
        .grant(
            h.vault,
            &folio_core::models::Principal::User(user.clone()),
            role,
            None,
        )
        .await
        .unwrap();
    h.vault_repository
        .set_default(&user, h.vault)
        .await
        .unwrap();
    let token = format!("source-session-{name}");
    h.auth_store
        .create_session(folio_auth::Session {
            id_hash: folio_auth::crypto::hash_token(&token),
            user_id: name.into(),
            expires_at: i64::MAX / 2,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    format!("folio_session={token}")
}

#[tokio::test]
async fn source_trash_and_restore_preserve_owner_authorization_and_server_actors() {
    let h = common::harness_requiring_login().await;
    let owner = source_session(&h, "source-owner", folio_core::models::VaultRole::Owner).await;
    let editor = source_session(&h, "source-editor", folio_core::models::VaultRole::Editor).await;
    let input: folio_core::models::IngestionSourceInput = serde_json::from_value(json!({
        "name": "Restricted source", "config": {"type": "folder", "path": "/unused/source"}
    }))
    .unwrap();
    let source = h
        .state
        .ingestion
        .managed
        .create(h.vault, input)
        .await
        .unwrap();
    h.state
        .ingestion
        .managed
        .delete(h.vault, source.id)
        .await
        .unwrap();
    let denied = h
        .app
        .clone()
        .oneshot(
            Request::post(format!("/api/sources/{}/restore", source.id))
                .header("cookie", &editor)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let deleted_list = h
        .app
        .clone()
        .oneshot(
            Request::get("/api/sources?deleted=true")
                .header("cookie", &editor)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted_list.status(), StatusCode::FORBIDDEN);
    let cross_origin = h
        .app
        .clone()
        .oneshot(
            Request::post(format!("/api/sources/{}/restore", source.id))
                .header("cookie", &owner)
                .header("origin", "https://untrusted.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cross_origin.status(), StatusCode::FORBIDDEN);
    let restored = h
        .app
        .clone()
        .oneshot(
            Request::post(format!("/api/sources/{}/restore", source.id))
                .header("cookie", &owner)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(restored.status(), StatusCode::OK);
    let body = common::text(restored).await;
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["updated_by"], "source-owner");
    assert!(body["deleted_at"].is_null());
}

#[tokio::test]
async fn source_outcomes_are_scoped_and_empty_before_first_attempt() {
    let source = configured_folder("/isolated/nonexistent-source");
    let id = source.id;
    let app = test_app(vec![source]).await;

    let (status, body) =
        json_request(&app, "GET", &format!("/api/sources/{id}/outcomes"), None).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["vault_id"], app.vault.to_string());
    assert_eq!(body["source_id"], id.to_string());
    assert_eq!(body["outcomes"], json!([]));
    assert!(body["next_before"].is_null());
    let (missing, _) = json_request(
        &app,
        "GET",
        &format!("/api/sources/{}/outcomes", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(missing, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn source_outcomes_reject_invalid_continuations() {
    let source = configured_folder("/isolated/nonexistent-source");
    let id = source.id;
    let app = test_app(vec![source]).await;
    let (status, _) = json_request(
        &app,
        "GET",
        &format!("/api/sources/{id}/outcomes?before=0"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = json_request(
        &app,
        "GET",
        &format!("/api/sources/{id}/outcomes?limit=101"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn source_outcomes_page_shows_empty_history_and_sanitized_health() {
    let source = configured_folder("/isolated/nonexistent-source");
    let id = source.id;
    let app = test_app(vec![source]).await;
    app.state
        .ingestion
        .managed
        .save_status(
            id,
            &folio_core::models::SourceStatus {
                last_error: Some("password=secret raw email body".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let (status, body) = send(
        &app,
        "GET",
        &format!("/settings/sources/{id}/outcomes"),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("No item outcomes yet"));
    assert!(body.contains("Source unavailable"));
    assert!(!body.contains("password=secret"));
    assert!(!body.contains("raw email body"));
}

#[tokio::test]
async fn source_outcomes_do_not_resolve_a_source_from_another_vault() {
    let source = configured_folder("/isolated/nonexistent-source");
    let id = source.id;
    let app = test_app(vec![source]).await;
    let other = app
        .vault_repository
        .create("Other vault", None)
        .await
        .unwrap();
    let response = app
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sources/{id}/outcomes"))
                .header("X-Folio-Vault", other.id.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

async fn seed_outcome(app: &Harness, source: Uuid, result: &str, document: Option<Uuid>) -> i64 {
    let folio_infra::Database::Sqlite(pool) = &app.database else {
        unreachable!()
    };
    let item = Uuid::new_v4();
    let attempt = Uuid::new_v4();
    sqlx::query("INSERT INTO ingest_items (id,vault_id,source_id,source_kind,source_managed,locator_key,attempt_id,started_at,lease_expires_at,decision_fingerprint,state) VALUES (?,?,?,'folder',1,?,?,'2026-11-02T00:00:00Z','2026-11-02T00:10:00Z',?,?)")
        .bind(item.to_string()).bind(app.vault.to_string()).bind(source.to_string())
        .bind(format!("{}{}", item.simple(), attempt.simple())).bind(attempt.to_string()).bind("a".repeat(64)).bind(result)
        .execute(pool).await.unwrap();
    sqlx::query_scalar("INSERT INTO ingest_outcomes (vault_id,source_id,item_id,attempt_id,event_id,occurred_at,outcome,document_id) VALUES (?,?,?,?,?,'2026-11-02T00:00:00Z',?,?) RETURNING id")
        .bind(app.vault.to_string()).bind(source.to_string()).bind(item.to_string()).bind(attempt.to_string())
        .bind(Uuid::new_v4().to_string()).bind(result).bind(document.map(|id| id.to_string()))
        .fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn source_outcomes_page_retains_history_without_linking_missing_documents() {
    let source = configured_folder("/isolated/nonexistent-source");
    let id = source.id;
    let app = test_app(vec![source]).await;
    let missing_document = Uuid::new_v4();
    let older = seed_outcome(&app, id, "imported", Some(missing_document)).await;
    let latest = seed_outcome(&app, id, "failed", None).await;

    let (status, page) = json_request(
        &app,
        "GET",
        &format!("/api/sources/{id}/outcomes?limit=1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["outcomes"][0]["id"], latest);
    assert_eq!(page["next_before"], latest);
    let (status, page) = json_request(
        &app,
        "GET",
        &format!("/api/sources/{id}/outcomes?before={latest}&limit=1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["outcomes"][0]["id"], older);
    assert_eq!(page["outcomes"][0]["outcome"], "imported");
    assert!(page["outcomes"][0]["document_id"].is_null());
    assert!(page["outcomes"][0].get("locator_key").is_none());
    assert!(page["next_before"].is_null());
    let (status, html) = send(
        &app,
        "GET",
        &format!("/settings/sources/{id}/outcomes"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Import failed"));
    assert!(html.contains("The document was saved"));
    assert!(!html.contains(&format!("/documents/{missing_document}")));
    let (status, html) = send(
        &app,
        "GET",
        &format!("/settings/sources/{id}/outcomes?limit=1"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(&format!("/outcomes?before={latest}&limit=1")));
    assert!(html.contains("/outcomes?limit=1\""));
}
