//! Paperless sources through the real router: settings, token handling and access.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use folio_core::models::SourceConfig;
use serde_json::{json, Value};
use tower::ServiceExt;

use common::Harness;

const TOKEN: &str = "paperless-secret-token";

async fn send(
    app: &Harness,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
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
    let text = common::text(response).await;
    let value = if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    };
    (status, value)
}

fn paperless_body(url: &str, token: &str) -> Value {
    json!({
        "name": "Paperless",
        "config": {"type": "paperless", "url": url, "token": token, "sync_mode": "mirror"}
    })
}

async fn stored_config(app: &Harness, id: &str) -> folio_core::models::PaperlessConfig {
    let def = app
        .state
        .ingestion
        .managed
        .find(app.vault, id.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let SourceConfig::Paperless(config) = def.config else {
        panic!("paperless")
    };
    config
}

#[tokio::test]
async fn tokens_are_redacted_and_a_placeholder_keeps_the_stored_one_for_the_same_url() {
    let app = common::harness().await;

    let (status, created) = send(
        &app,
        "POST",
        "/api/sources",
        None,
        Some(paperless_body("https://paperless.example", TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["config"]["token"], "********");
    assert_eq!(created["config"]["sync_mode"], "mirror");
    assert_eq!(created["config"]["poll_interval_secs"], 900);
    let id = created["id"].as_str().unwrap().to_string();

    for uri in ["/api/sources".to_string(), format!("/api/sources/{id}")] {
        let (_, listed) = send(&app, "GET", &uri, None, None).await;
        assert!(!listed.to_string().contains(TOKEN), "{uri}: {listed}");
    }

    let (status, updated) = send(
        &app,
        "PUT",
        &format!("/api/sources/{id}"),
        None,
        Some(json!({
            "name": "Paperless",
            "config": {"type": "paperless", "url": "https://paperless.example", "token": "********", "sync_mode": "new_only"}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["config"]["token"], "********");
    let stored = stored_config(&app, &id).await;
    assert_eq!(stored.token.as_deref(), Some(TOKEN));
    assert_eq!(
        stored.sync_mode,
        folio_core::models::PaperlessSyncMode::NewOnly
    );
}

#[tokio::test]
async fn a_new_url_with_the_placeholder_asks_for_the_token_and_changes_nothing() {
    let app = common::harness().await;
    let (_, created) = send(
        &app,
        "POST",
        "/api/sources",
        None,
        Some(paperless_body("https://paperless.example", TOKEN)),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();

    for token in [json!("********"), json!(""), Value::Null] {
        let mut body = paperless_body("https://elsewhere.example", "");
        body["config"]["token"] = token.clone();
        let (status, error) =
            send(&app, "PUT", &format!("/api/sources/{id}"), None, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{token}: {error}");
        assert!(
            error["error"].as_str().unwrap().contains("token"),
            "{error}"
        );
    }
    let stored = stored_config(&app, &id).await;
    assert_eq!(stored.url, "https://paperless.example");
    assert_eq!(stored.token.as_deref(), Some(TOKEN));

    let (status, _) = send(
        &app,
        "PUT",
        &format!("/api/sources/{id}"),
        None,
        Some(paperless_body("https://elsewhere.example", "new-token")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let stored = stored_config(&app, &id).await;
    assert_eq!(stored.url, "https://elsewhere.example");
    assert_eq!(stored.token.as_deref(), Some("new-token"));
}

#[tokio::test]
async fn invalid_settings_are_rejected_naming_the_field() {
    let app = common::harness().await;
    let cases = [
        (
            "url",
            json!({"url": "ftp://paperless.example", "token": TOKEN}),
        ),
        ("url", json!({"url": "not a url", "token": TOKEN})),
        (
            "poll_interval_secs",
            json!({"url": "https://p.example", "token": TOKEN, "poll_interval_secs": 30}),
        ),
        (
            "token_file",
            json!({"url": "https://p.example", "token_file": "/etc/passwd"}),
        ),
        (
            "token_file",
            json!({"url": "https://p.example", "token": TOKEN, "token_file": "/etc/passwd"}),
        ),
        ("token", json!({"url": "https://p.example"})),
        (
            "default_currency",
            json!({"url": "https://p.example", "token": TOKEN, "default_currency": "euro"}),
        ),
    ];
    for (field, mut config) in cases {
        config["type"] = json!("paperless");
        let body = json!({"name": "Paperless", "config": config});
        let (status, error) = send(&app, "POST", "/api/sources", None, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {error}");
        assert!(
            error["error"].as_str().unwrap().contains(field),
            "{field}: {error}"
        );
    }
    let (_, listed) = send(&app, "GET", "/api/sources", None, None).await;
    assert!(listed.as_array().unwrap().is_empty(), "{listed}");

    let (_, created) = send(
        &app,
        "POST",
        "/api/sources",
        None,
        Some(paperless_body("https://paperless.example", TOKEN)),
    )
    .await;
    let id = created["id"].as_str().unwrap();
    let mut body = paperless_body("https://paperless.example", "********");
    body["config"]["token_file"] = json!("/etc/passwd");
    let (status, error) = send(&app, "PUT", &format!("/api/sources/{id}"), None, Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error["error"].as_str().unwrap().contains("token_file"));
}

/// A TCP listener standing in for Paperless that only counts connections
async fn paperless_listener() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            drop(socket);
        }
    });
    (url, hits)
}

/// A signed-in account that owns the harness vault
async fn vault_owner_session(h: &Harness, name: &str, is_admin: bool) -> String {
    h.auth_store
        .create_user(folio_auth::User {
            id: name.into(),
            username: name.into(),
            password_hash: "!".into(),
            created_at: 1,
            email: None,
            display_name: None,
            is_admin,
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
            folio_core::models::VaultRole::Owner,
            None,
        )
        .await
        .unwrap();
    h.vault_repository
        .set_default(&user, h.vault)
        .await
        .unwrap();
    let token = format!("paperless-session-{name}");
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

async fn seed_paperless(
    h: &Harness,
    name: &str,
    url: &str,
) -> folio_core::models::IngestionSourceDef {
    let input: folio_core::models::IngestionSourceInput = serde_json::from_value(json!({
        "name": name,
        "config": {"type": "paperless", "url": url, "token": TOKEN}
    }))
    .unwrap();
    h.state
        .ingestion
        .managed
        .create(h.vault, input)
        .await
        .unwrap()
}

async fn send_form(h: &Harness, uri: &str, cookie: &str, form: &str) -> StatusCode {
    h.app
        .clone()
        .oneshot(
            Request::post(uri)
                .header("cookie", cookie)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(form.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_vault_owner_who_is_not_an_admin_is_forbidden_every_paperless_operation() {
    let h = common::harness_requiring_login().await;
    let (url, hits) = paperless_listener().await;
    let cookie = vault_owner_session(&h, "owner-not-admin", false).await;
    let live = seed_paperless(&h, "Live", &url).await;
    let retained = seed_paperless(&h, "Retained", &url).await;
    h.state
        .ingestion
        .managed
        .delete(h.vault, retained.id)
        .await
        .unwrap();
    let (live_id, retained_id) = (live.id, retained.id);

    let body = paperless_body(&url, "another-token");
    let json_calls = [
        ("POST", "/api/sources".to_string(), Some(body.clone())),
        ("PUT", format!("/api/sources/{live_id}"), Some(body.clone())),
        ("DELETE", format!("/api/sources/{live_id}"), None),
        ("POST", format!("/api/sources/{retained_id}/restore"), None),
        ("POST", format!("/api/sources/{live_id}/test"), None),
    ];
    for (method, uri, body) in json_calls {
        let (status, error) = send(&h, method, &uri, Some(&cookie), body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {error}");
        assert_eq!(error["code"], "forbidden", "{method} {uri}: {error}");
    }

    let form = format!(
        "kind=paperless&name=Paperless&enabled=on&url={url}&token=t&sync_mode=mirror\
&poll_interval_secs=900&default_currency=EUR"
    );
    let html_calls = [
        ("/settings/sources".to_string(), form.clone()),
        ("/settings/sources".to_string(), format!("{form}&test=1")),
        (format!("/settings/sources/{live_id}"), form.clone()),
        (
            format!("/settings/sources/{live_id}"),
            format!("{form}&test=1"),
        ),
        (format!("/settings/sources/{live_id}/toggle"), String::new()),
        (format!("/settings/sources/{live_id}/delete"), String::new()),
        (
            format!("/settings/sources/{retained_id}/restore"),
            String::new(),
        ),
        (format!("/settings/sources/{live_id}/test"), String::new()),
    ];
    for (uri, form) in html_calls {
        assert_eq!(
            send_form(&h, &uri, &cookie, &form).await,
            StatusCode::FORBIDDEN,
            "{uri} {form}"
        );
    }
    for uri in [
        "/settings/sources/new?kind=paperless".to_string(),
        format!("/settings/sources/{live_id}"),
    ] {
        let response = h
            .app
            .clone()
            .oneshot(
                Request::get(&uri)
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
    }

    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    let active = h.state.ingestion.managed.list(h.vault).await.unwrap();
    assert_eq!(active, vec![live.clone()]);
    let deleted = h
        .state
        .ingestion
        .managed
        .list_deleted(h.vault)
        .await
        .unwrap();
    assert_eq!(
        deleted.iter().map(|d| d.id).collect::<Vec<_>>(),
        [retained_id]
    );

    let folder = json!({"name": "Scans", "config": {"type": "folder", "path": "/unused/scans"}});
    let (status, _) = send(&h, "POST", "/api/sources", Some(&cookie), Some(folder)).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "other kinds keep the owner rule"
    );
}

#[tokio::test]
async fn an_admin_who_owns_the_vault_manages_paperless_sources() {
    let h = common::harness_requiring_login().await;
    let (url, hits) = paperless_listener().await;
    let cookie = vault_owner_session(&h, "admin-owner", true).await;

    let (status, created) = send(
        &h,
        "POST",
        "/api/sources",
        Some(&cookie),
        Some(paperless_body(&url, TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["config"]["token"], "********");
    let id = created["id"].as_str().unwrap();

    let (status, tested) = send(
        &h,
        "POST",
        &format!("/api/sources/{id}/test"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tested, json!({"ok": false, "message": "unsupported"}));

    assert_eq!(
        send_form(&h, &format!("/settings/sources/{id}/toggle"), &cookie, "").await,
        StatusCode::SEE_OTHER
    );
    let (status, _) = send(
        &h,
        "DELETE",
        &format!("/api/sources/{id}"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, restored) = send(
        &h,
        "POST",
        &format!("/api/sources/{id}/restore"),
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(restored["enabled"], false);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_vault_has_at_most_one_live_paperless_source() {
    let app = common::harness().await;
    let create = |url: &'static str| {
        let app = &app;
        async move {
            send(
                app,
                "POST",
                "/api/sources",
                None,
                Some(paperless_body(url, TOKEN)),
            )
            .await
        }
    };

    let (status, first) = create("https://one.example").await;
    assert_eq!(status, StatusCode::CREATED);
    let first_id = first["id"].as_str().unwrap().to_string();
    let (status, error) = create("https://two.example").await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");

    let folder = json!({"name": "Scans", "config": {"type": "folder", "path": "/unused/scans"}});
    let (_, folder) = send(&app, "POST", "/api/sources", None, Some(folder)).await;
    let folder_id = folder["id"].as_str().unwrap();
    let (status, error) = send(
        &app,
        "PUT",
        &format!("/api/sources/{folder_id}"),
        None,
        Some(paperless_body("https://two.example", TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");

    let (status, _) = send(
        &app,
        "PUT",
        &format!("/api/sources/{first_id}"),
        None,
        Some(paperless_body("https://one.example", "********")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a source keeps its own slot");

    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/sources/{first_id}"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, second) = create("https://two.example").await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    let (status, error) = send(
        &app,
        "POST",
        &format!("/api/sources/{first_id}/restore"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    let response = app
        .app
        .clone()
        .oneshot(
            Request::post(format!("/settings/sources/{first_id}/restore"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_paperless_source_from_the_configuration_wins_over_a_managed_one() {
    let mut app = common::harness().await;
    let configured: folio_core::models::IngestionSourceDef = serde_json::from_value(json!({
        "id": uuid::Uuid::new_v4(),
        "name": "home",
        "enabled": true,
        "origin": "config",
        "vault_id": app.vault,
        "config": {"type": "paperless", "url": "https://paperless.example", "token_file": "/run/secrets/t"}
    }))
    .unwrap();
    let mut state = app.state.clone();
    state.ingestion.configured = std::sync::Arc::new(vec![configured]);
    app.app = folio_api::create_router(
        state,
        folio_api::AuthState::new(app.auth_store.clone(), None).with_required(false),
    );

    let (status, listed) = send(&app, "GET", "/api/sources", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed[0]["origin"], "config");
    assert_eq!(listed[0]["config"]["type"], "paperless");

    let (status, error) = send(
        &app,
        "POST",
        "/api/sources",
        None,
        Some(paperless_body("https://other.example", TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("server configuration"));
}
