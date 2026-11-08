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
    assert_eq!(stored.url, "https://paperless.example/");
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
    assert_eq!(stored.url, "https://elsewhere.example/");
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

/// A TCP listener at the source URL that counts connections, so a guard that
/// lets a request through to the Paperless client fails the test
async fn paperless_tripwire() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
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

async fn vault_owner_session(h: &Harness, name: &str, is_admin: bool) -> String {
    common::vault_session(h, name, folio_core::models::VaultRole::Owner, is_admin).await
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

#[tokio::test]
async fn a_vault_owner_who_is_not_an_admin_is_forbidden_every_paperless_operation() {
    let h = common::harness_requiring_login().await;
    let (url, hits) = paperless_tripwire().await;
    let cookie = vault_owner_session(&h, "owner-not-admin", false).await;
    let retained = seed_paperless(&h, "Retained", &url).await;
    h.state
        .ingestion
        .managed
        .delete(h.vault, retained.id)
        .await
        .unwrap();
    let live = seed_paperless(&h, "Live", &url).await;
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
            page(&h, "POST", &uri, Some(&cookie), Some(&form)).await.0,
            StatusCode::FORBIDDEN,
            "{uri} {form}"
        );
    }
    for uri in [
        "/settings/sources/new?kind=paperless".to_string(),
        format!("/settings/sources/{live_id}"),
    ] {
        let (status, _) = page(&h, "GET", &uri, Some(&cookie), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
    }

    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "nothing reached the Paperless URL"
    );
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
    let (url, hits) = paperless_tripwire().await;
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
        page(
            &h,
            "POST",
            &format!("/settings/sources/{id}/toggle"),
            Some(&cookie),
            Some("")
        )
        .await
        .0,
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
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "nothing reached the Paperless URL"
    );
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
    let restore = format!("/settings/sources/{first_id}/restore");
    let (status, _) = page(&app, "POST", &restore, None, None).await;
    assert_eq!(status, StatusCode::CONFLICT);
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

async fn page(
    app: &Harness,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    form: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if form.is_some() {
        builder = builder.header("content-type", "application/x-www-form-urlencoded");
    }
    let response = app
        .app
        .clone()
        .oneshot(
            builder
                .body(Body::from(form.unwrap_or_default().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (status, common::text(response).await)
}

#[tokio::test]
async fn the_paperless_form_renders_saves_and_never_shows_the_token() {
    let app = common::harness().await;

    let (status, list) = page(&app, "GET", "/settings/sources", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        list.contains("/settings/sources/new?kind=paperless"),
        "{list}"
    );

    let (status, blank) = page(
        &app,
        "GET",
        "/settings/sources/new?kind=paperless",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for needle in [
        r#"name="kind" value="paperless""#,
        r#"type="url" id="url" name="url""#,
        r#"type="password" id="token" name="token""#,
        r#"name="sync_mode" value="mirror" checked"#,
        r#"<option value="900" selected>Every 15 minutes</option>"#,
        r#"id="default_currency""#,
        "Save and start first sync",
    ] {
        assert!(blank.contains(needle), "missing {needle}");
    }

    let form = format!(
        "kind=paperless&name=Paperless&enabled=on&url=https%3A%2F%2Fpaperless.example\
&token={TOKEN}&sync_mode=new_only&poll_interval_secs=3600&default_currency=eur"
    );
    let (status, body) = page(&app, "POST", "/settings/sources", None, Some(&form)).await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{body}");
    let def = app
        .state
        .ingestion
        .managed
        .list(app.vault)
        .await
        .unwrap()
        .remove(0);
    let SourceConfig::Paperless(saved) = &def.config else {
        panic!("paperless")
    };
    assert_eq!(saved.url, "https://paperless.example/");
    assert_eq!(saved.token.as_deref(), Some(TOKEN));
    assert_eq!(
        saved.sync_mode,
        folio_core::models::PaperlessSyncMode::NewOnly
    );
    assert_eq!(saved.poll_interval_secs, 3600);
    assert_eq!(saved.default_currency.as_deref(), Some("EUR"));

    let (_, list) = page(&app, "GET", "/settings/sources", None, None).await;
    assert!(list.contains("https://paperless.example"));
    assert!(
        !list.contains("new?kind=paperless"),
        "a vault has one Paperless source"
    );

    let edit_uri = format!("/settings/sources/{}", def.id);
    let (status, edit) = page(&app, "GET", &edit_uri, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!edit.contains(TOKEN));
    assert!(edit.contains(r#"value="********""#));
    assert!(edit.contains(r#"name="sync_mode" value="new_only" checked"#));
    assert!(edit.contains(r#"<option value="3600" selected>Every hour</option>"#));

    let masked = "kind=paperless&name=Paperless&enabled=on&url=https%3A%2F%2Fpaperless.example\
&token=********&sync_mode=mirror&poll_interval_secs=900&default_currency=EUR";
    let (status, _) = page(&app, "POST", &edit_uri, None, Some(masked)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let SourceConfig::Paperless(kept) = app
        .state
        .ingestion
        .managed
        .find(app.vault, def.id)
        .await
        .unwrap()
        .unwrap()
        .config
    else {
        panic!("paperless")
    };
    assert_eq!(kept.token.as_deref(), Some(TOKEN));
    assert_eq!(
        kept.sync_mode,
        folio_core::models::PaperlessSyncMode::Mirror
    );

    let moved = masked.replace("paperless.example", "elsewhere.example");
    let (status, refused) = page(&app, "POST", &edit_uri, None, Some(&moved)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(refused.contains("enter the API token again"), "{refused}");
    assert!(refused.contains(r#"name="token" value="" autocomplete="new-password" required"#));

    let second = "kind=paperless&name=Second&enabled=on&url=https%3A%2F%2Fsecond.example\
&token=t&sync_mode=mirror&poll_interval_secs=900";
    let (status, conflict) = page(&app, "POST", "/settings/sources", None, Some(second)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        conflict.contains("already has a Paperless source"),
        "{conflict}"
    );

    let (status, tested) = page(
        &app,
        "POST",
        &format!("/settings/sources/{}/test", def.id),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(tested.contains("Paperless: unsupported"));
}

#[tokio::test]
async fn non_admin_owners_see_the_paperless_source_without_its_controls() {
    let h = common::harness_requiring_login().await;
    let cookie = vault_owner_session(&h, "viewer-owner", false).await;
    let def = seed_paperless(&h, "Library", "https://paperless.example").await;
    let (status, list) = page(&h, "GET", "/settings/sources", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.contains("Library"));
    assert!(list.contains("Only an instance admin can change it"));
    assert!(!list.contains(&format!("/{}/test", def.id)));
    assert!(!list.contains(&format!("/{}/toggle", def.id)));
    assert!(!list.contains("new?kind=paperless"));
}

fn paperless_form(url: &str, token: &str) -> String {
    format!(
        "kind=paperless&name=Paperless&enabled=on&url={}&token={token}&sync_mode=mirror\
&poll_interval_secs=900&default_currency=EUR",
        url.replace(':', "%3A").replace('/', "%2F")
    )
}

#[tokio::test]
async fn re_rendered_forms_never_echo_the_token() {
    let app = common::harness().await;
    let typed = "typed-secret-token";

    let (status, tested) = page(
        &app,
        "POST",
        "/settings/sources",
        None,
        Some(&format!(
            "{}&test=1",
            paperless_form("https://p.example", typed)
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!tested.contains(typed), "try-connection echoed the token");
    assert!(tested.contains(r#"name="token" value="" autocomplete="new-password""#));

    let invalid = paperless_form("https://p.example", typed).replace("EUR", "euro");
    let (_, refused) = page(&app, "POST", "/settings/sources", None, Some(&invalid)).await;
    assert!(refused.contains("default_currency"), "{refused}");
    assert!(!refused.contains(typed), "the error page echoed the token");

    let (status, _) = page(
        &app,
        "POST",
        "/settings/sources",
        None,
        Some(&paperless_form("https://p.example", TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (_, conflict) = page(
        &app,
        "POST",
        "/settings/sources",
        None,
        Some(&paperless_form("https://other.example", typed)),
    )
    .await;
    assert!(conflict.contains("already has a Paperless source"));
    assert!(!conflict.contains(typed), "the 409 page echoed the token");

    let id = app.state.ingestion.managed.list(app.vault).await.unwrap()[0].id;
    let edit = format!("/settings/sources/{id}");
    let (_, same_url) = page(
        &app,
        "POST",
        &edit,
        None,
        Some(&format!(
            "{}&test=1",
            paperless_form("https://P.example", typed)
        )),
    )
    .await;
    assert!(!same_url.contains(typed) && !same_url.contains(TOKEN));
    assert!(same_url.contains(r#"name="token" value="********""#));
    let (_, new_url) = page(
        &app,
        "POST",
        &edit,
        None,
        Some(&format!(
            "{}&test=1",
            paperless_form("https://new.example", typed)
        )),
    )
    .await;
    assert!(!new_url.contains(typed) && !new_url.contains(TOKEN));
    assert!(new_url.contains(r#"name="token" value="""#));
}

#[tokio::test]
async fn urls_are_stored_normalized_and_compared_that_way() {
    let app = common::harness().await;
    let (status, created) = send(
        &app,
        "POST",
        "/api/sources",
        None,
        Some(paperless_body("  HTTPS://Paperless.Example ", TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["config"]["url"], "https://paperless.example/");
    let id = created["id"].as_str().unwrap();

    let (status, _) = send(
        &app,
        "PUT",
        &format!("/api/sources/{id}"),
        None,
        Some(paperless_body("https://paperless.example", "********")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the same URL keeps the token");

    let long = format!("https://p.example/{}", "a".repeat(2048));
    for url in [long.as_str(), "https://p.example/\u{7}"] {
        let (status, error) = send(
            &app,
            "PUT",
            &format!("/api/sources/{id}"),
            None,
            Some(paperless_body(url, "new-token")),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
        assert!(error["error"].as_str().unwrap().contains("url"));
    }
}

#[tokio::test]
async fn restoring_a_source_that_is_not_in_the_trash_is_not_found() {
    let app = common::harness().await;
    let (_, created) = send(
        &app,
        "POST",
        "/api/sources",
        None,
        Some(paperless_body("https://p.example", TOKEN)),
    )
    .await;
    for id in [
        created["id"].as_str().unwrap().to_string(),
        uuid::Uuid::new_v4().to_string(),
    ] {
        let (status, _) = send(
            &app,
            "POST",
            &format!("/api/sources/{id}/restore"),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{id}");
    }
}

#[tokio::test]
async fn a_non_admin_cannot_switch_a_source_into_or_out_of_paperless() {
    let h = common::harness_requiring_login().await;
    let cookie = vault_owner_session(&h, "switch-kind", false).await;
    let mail = json!({
        "name": "Mail",
        "config": {"type": "email", "host": "imap.example.test", "username": "me", "password": "pw"}
    });
    let (status, created) = send(&h, "POST", "/api/sources", Some(&cookie), Some(mail)).await;
    assert_eq!(status, StatusCode::CREATED);
    let mail_id = created["id"].as_str().unwrap();
    let (status, error) = send(
        &h,
        "PUT",
        &format!("/api/sources/{mail_id}"),
        Some(&cookie),
        Some(paperless_body("https://p.example", TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error["code"], "forbidden");

    let library = seed_paperless(&h, "Library", "https://p.example").await;
    let back_to_mail = json!({
        "name": "Library",
        "config": {"type": "email", "host": "imap.example.test", "username": "me", "password": "pw"}
    });
    let (status, error) = send(
        &h,
        "PUT",
        &format!("/api/sources/{}", library.id),
        Some(&cookie),
        Some(back_to_mail),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error["code"], "forbidden");
    let stored = h
        .state
        .ingestion
        .managed
        .find(h.vault, library.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored, library);
}

#[tokio::test]
async fn an_admin_needs_to_own_the_vault_selected_by_header_or_prefix() {
    let h = common::harness_requiring_login().await;
    let cookie = common::vault_session(
        &h,
        "admin-editor",
        folio_core::models::VaultRole::Owner,
        true,
    )
    .await;
    let other = h.vault_repository.create("Shared", None).await.unwrap().id;
    h.members
        .grant(
            other,
            &folio_core::models::Principal::User("admin-editor".into()),
            folio_core::models::VaultRole::Editor,
            None,
        )
        .await
        .unwrap();

    let request = |vault: String| {
        Request::post("/api/sources")
            .header("cookie", &cookie)
            .header("x-folio-vault", vault)
            .header("content-type", "application/json")
            .body(Body::from(
                paperless_body("https://p.example", TOKEN).to_string(),
            ))
            .unwrap()
    };
    let denied = h
        .app
        .clone()
        .oneshot(request(other.to_string()))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let (status, _) = page(
        &h,
        "POST",
        &format!("/vaults/{other}/settings/sources"),
        Some(&cookie),
        Some(&paperless_form("https://p.example", TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(h
        .state
        .ingestion
        .managed
        .list(other)
        .await
        .unwrap()
        .is_empty());

    let allowed = h
        .app
        .clone()
        .oneshot(request(h.vault.to_string()))
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn with_auth_disabled_a_signed_in_member_counts_as_an_admin() {
    let h = common::harness().await;
    let cookie = vault_owner_session(&h, "local-user", false).await;
    let (status, created) = send(
        &h,
        "POST",
        "/api/sources",
        Some(&cookie),
        Some(paperless_body("https://p.example", TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
}

#[tokio::test]
async fn bearer_tokens_need_the_vault_management_scope_for_paperless() {
    let h = common::harness_requiring_login().await;
    vault_owner_session(&h, "token-admin", true).await;
    h.auth_store
        .insert_client(folio_auth::OAuthClient {
            client_id: "paperless-client".into(),
            client_name: "Paperless test client".into(),
            redirect_uris: vec!["http://localhost/callback".into()],
            created_at: 1,
        })
        .await
        .unwrap();
    for (token, scopes) in [
        ("write-only", vec!["documents:write"]),
        ("manager", vec!["documents:write", "vaults:manage"]),
    ] {
        h.auth_store
            .insert_access_token(folio_auth::AccessToken {
                token_hash: folio_auth::crypto::hash_token(token),
                client_id: "paperless-client".into(),
                user_id: "token-admin".into(),
                scopes: scopes.into_iter().map(String::from).collect(),
                audience: "http://localhost/api".into(),
                expires_at: i64::MAX,
                grant_id: format!("grant-{token}"),
            })
            .await
            .unwrap();
    }
    let post = |token: &str, body: Value| {
        Request::post("/api/sources")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let folder = json!({"name": "Scans", "config": {"type": "folder", "path": "/unused/scans"}});

    let refused = h
        .app
        .clone()
        .oneshot(post(
            "write-only",
            paperless_body("https://p.example", TOKEN),
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let body: Value = serde_json::from_str(&common::text(refused).await).unwrap();
    assert_eq!(body["code"], "insufficient_scope");
    let folder_ok = h
        .app
        .clone()
        .oneshot(post("write-only", folder))
        .await
        .unwrap();
    assert_eq!(
        folder_ok.status(),
        StatusCode::CREATED,
        "other kinds keep their scope"
    );

    let created = h
        .app
        .clone()
        .oneshot(post("manager", paperless_body("https://p.example", TOKEN)))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
}
