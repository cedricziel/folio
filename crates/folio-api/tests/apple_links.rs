//! `/.well-known/apple-app-site-association` and the Apple app links setting

mod common;

use axum::http::{header, StatusCode};
use axum::Router;
use common::*;
use folio_api::AuthState;
use folio_core::models::AppleAppId;
use tower::ServiceExt;

const AASA: &str = "/.well-known/apple-app-site-association";

fn app_with(h: &Harness, app_ids: &[&str], required: bool) -> Router {
    let ids = app_ids
        .iter()
        .map(|id| id.parse().unwrap())
        .collect::<Vec<AppleAppId>>();
    folio_api::create_router(
        h.state.clone().with_apple_app_ids_default(ids),
        AuthState::new(h.auth_store.clone(), None).with_required(required),
    )
}

#[tokio::test]
async fn association_is_absent_without_app_ids() {
    let h = harness().await;
    let response = app_with(&h, &[], false).oneshot(get(AASA)).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unconfigured_association_is_404_not_a_login_redirect() {
    let h = harness_requiring_login().await;
    let response = app_with(&h, &[], true).oneshot(get(AASA)).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.headers().get(header::LOCATION).is_none());
}

const U: &str = "????????-????-????-????-????????????";

fn app_with_public_url(h: &Harness, app_ids: &[&str], public_url: &str) -> Router {
    let ids = app_ids
        .iter()
        .map(|id| id.parse().unwrap())
        .collect::<Vec<AppleAppId>>();
    let mut state = h.state.clone().with_apple_app_ids_default(ids);
    state.public_url = public_url.into();
    folio_api::create_router(
        state,
        AuthState::new(h.auth_store.clone(), None).with_required(true),
    )
}

async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_str(&text(response).await).unwrap()
}

#[tokio::test]
async fn configured_association_lists_app_ids_and_the_three_page_shapes() {
    let h = harness_requiring_login().await;
    let app = app_with(&h, &["ABCDE12345.com.example.folio"], true);
    let response = app.oneshot(get(AASA)).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    assert_eq!(headers[header::CACHE_CONTROL], "public, max-age=3600");
    assert!(headers.get(header::SET_COOKIE).is_none());

    let body = json(response).await;
    assert_eq!(
        body,
        serde_json::json!({
            "applinks": {
                "details": [{
                    "appIDs": ["ABCDE12345.com.example.folio"],
                    "components": [
                        { "/": format!("/vaults/{U}/documents/{U}") },
                        { "/": format!("/vaults/{U}/saved-views/{U}") },
                        { "/": format!("/vaults/{U}/") },
                    ]
                }]
            }
        })
    );
    let raw = body.to_string();
    assert!(!raw.contains('*'), "{raw}");
    assert!(!raw.contains("exclude"), "{raw}");
}

#[tokio::test]
async fn association_patterns_carry_the_public_url_path_prefix() {
    let h = harness_requiring_login().await;
    for public_url in ["https://example.com/folio", "https://example.com/folio/"] {
        let app = app_with_public_url(&h, &["ABCDE12345.com.example.folio"], public_url);
        let body = json(app.oneshot(get(AASA)).await.unwrap()).await;
        let components = body["applinks"]["details"][0]["components"]
            .as_array()
            .unwrap();
        assert_eq!(components.len(), 3);
        for component in components {
            let pattern = component["/"].as_str().unwrap();
            assert!(
                pattern.starts_with("/folio/vaults/"),
                "{public_url}: {pattern}"
            );
        }
    }
}

#[tokio::test]
async fn legacy_root_path_is_not_served() {
    let h = harness().await;
    let app = app_with(&h, &["ABCDE12345.com.example.folio"], false);
    let response = app
        .oneshot(get("/apple-app-site-association"))
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::OK);
    assert!(!text(response).await.contains("applinks"));
}

async fn served_app_ids(app: &Router) -> Option<Vec<String>> {
    let response = app.clone().oneshot(get(AASA)).await.unwrap();
    if response.status() == StatusCode::NOT_FOUND {
        return None;
    }
    let body = json(response).await;
    Some(
        body["applinks"]["details"][0]["appIDs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_string())
            .collect(),
    )
}

#[tokio::test]
async fn settings_row_replaces_config_without_a_restart_and_empty_falls_back() {
    let h = harness().await;
    let app = app_with(&h, &["ABCDE12345.com.example.folio"], false);
    let settings = &h.state.settings_repo;

    settings
        .set(
            "apple.app_ids",
            "FGHIJ67890.org.example.one\nFGHIJ67890.org.example.two",
        )
        .await
        .unwrap();
    assert_eq!(
        served_app_ids(&app).await.unwrap(),
        ["FGHIJ67890.org.example.one", "FGHIJ67890.org.example.two"]
    );

    settings.set("apple.app_ids", "").await.unwrap();
    assert_eq!(
        served_app_ids(&app).await.unwrap(),
        ["ABCDE12345.com.example.folio"]
    );
}

#[tokio::test]
async fn settings_row_alone_publishes_the_association() {
    let h = harness().await;
    let app = app_with(&h, &[], false);
    assert_eq!(served_app_ids(&app).await, None);
    h.state
        .settings_repo
        .set("apple.app_ids", "FGHIJ67890.org.example.one")
        .await
        .unwrap();
    assert_eq!(
        served_app_ids(&app).await.unwrap(),
        ["FGHIJ67890.org.example.one"]
    );
}

fn post_form(uri: &str, cookie: Option<&str>, body: &str) -> axum::http::Request<axum::body::Body> {
    let mut request = axum::http::Request::post(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    request
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

fn form_encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

async fn stored(h: &Harness) -> Option<String> {
    h.state
        .settings_repo
        .get("apple.app_ids")
        .await
        .unwrap()
        .map(|s| s.value)
}

#[tokio::test]
async fn admin_saves_app_ids_one_per_line() {
    let h = harness_requiring_login().await;
    let admin = vault_session(&h, "admin", folio_core::models::VaultRole::Owner, true).await;
    let app = app_with(&h, &[], true);
    let body = format!(
        "app_ids={}",
        form_encode(" ABCDE12345.com.example.folio\r\nABCDE12345.com.example.beta \r\n")
    );
    let response = app
        .oneshot(post_form("/settings/apple", Some(&admin), &body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers()[header::LOCATION],
        "/settings?saved=apple"
    );
    assert_eq!(
        stored(&h).await.as_deref(),
        Some("ABCDE12345.com.example.folio\nABCDE12345.com.example.beta")
    );
}

#[tokio::test]
async fn invalid_line_is_rejected_with_422_and_nothing_is_stored() {
    let h = harness_requiring_login().await;
    let admin = vault_session(&h, "admin", folio_core::models::VaultRole::Owner, true).await;
    h.state
        .settings_repo
        .set("apple.app_ids", "FGHIJ67890.org.example.kept")
        .await
        .unwrap();
    let app = app_with(&h, &[], true);
    let body = format!(
        "app_ids={}",
        form_encode("ABCDE12345.com.example.folio\nnot-an-app-id")
    );
    let response = app
        .oneshot(post_form("/settings/apple", Some(&admin), &body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let page = text(response).await;
    assert!(page.contains("not-an-app-id"), "{page}");
    assert!(page.contains("Apple app links"), "{page}");
    assert_eq!(
        stored(&h).await.as_deref(),
        Some("FGHIJ67890.org.example.kept")
    );
}

#[tokio::test]
async fn non_admin_cannot_save_app_ids() {
    let h = harness_requiring_login().await;
    let member = vault_session(&h, "member", folio_core::models::VaultRole::Editor, false).await;
    let app = app_with(&h, &[], true);
    let response = app
        .oneshot(post_form(
            "/settings/apple",
            Some(&member),
            "app_ids=ABCDE12345.com.example.folio",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(stored(&h).await, None);
}

async fn settings_page(app: &Router) -> String {
    let response = app.clone().oneshot(get("/settings")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    text(response).await
}

#[tokio::test]
async fn settings_page_shows_effective_app_ids_their_source_and_the_cdn_note() {
    let h = harness().await;

    let page = settings_page(&app_with(&h, &[], false)).await;
    assert!(page.contains("Apple app links"), "{page}");
    assert!(page.contains("not configured"), "{page}");
    assert!(
        page.contains("http://folio.test/.well-known/apple-app-site-association"),
        "{page}"
    );
    assert!(
        page.contains("Apple&#x27;s CDN") || page.contains("Apple's CDN"),
        "{page}"
    );

    let page = settings_page(&app_with(&h, &["ABCDE12345.com.example.folio"], false)).await;
    assert!(page.contains("from config.toml"), "{page}");
    assert!(page.contains("ABCDE12345.com.example.folio"), "{page}");

    h.state
        .settings_repo
        .set("apple.app_ids", "FGHIJ67890.org.example.one")
        .await
        .unwrap();
    let page = settings_page(&app_with(&h, &["ABCDE12345.com.example.folio"], false)).await;
    assert!(page.contains("from settings"), "{page}");
    assert!(page.contains("FGHIJ67890.org.example.one"), "{page}");
}

#[tokio::test]
async fn saved_notice_names_apple_app_links() {
    let h = harness().await;
    let response = app_with(&h, &[], false)
        .oneshot(get("/settings?saved=apple"))
        .await
        .unwrap();
    assert!(text(response).await.contains("Apple app links saved."));
}

#[tokio::test]
async fn clearing_the_field_falls_back_to_config() {
    let h = harness().await;
    h.state
        .settings_repo
        .set("apple.app_ids", "FGHIJ67890.org.example.one")
        .await
        .unwrap();
    let app = app_with(&h, &["ABCDE12345.com.example.folio"], false);
    let response = app
        .clone()
        .oneshot(post_form("/settings/apple", None, "app_ids="))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        served_app_ids(&app).await.unwrap(),
        ["ABCDE12345.com.example.folio"]
    );
    assert!(settings_page(&app).await.contains("from config.toml"));
}

#[tokio::test]
async fn invalid_stored_row_is_shown_as_an_error_and_config_still_serves() {
    let h = harness().await;
    h.state
        .settings_repo
        .set("apple.app_ids", "not-an-app-id")
        .await
        .unwrap();
    let app = app_with(&h, &["ABCDE12345.com.example.folio"], false);
    assert_eq!(
        served_app_ids(&app).await.unwrap(),
        ["ABCDE12345.com.example.folio"]
    );
    let page = settings_page(&app).await;
    assert!(page.contains("f-notice-danger"), "{page}");
    assert!(page.contains("not-an-app-id"), "{page}");
    assert!(page.contains("from config.toml"), "{page}");
}

#[tokio::test]
async fn cross_origin_post_with_an_admin_session_is_rejected() {
    let h = harness_requiring_login().await;
    let admin = vault_session(&h, "admin", folio_core::models::VaultRole::Owner, true).await;
    let app = app_with(&h, &[], true);
    let mut request = post_form(
        "/settings/apple",
        Some(&admin),
        "app_ids=ABCDE12345.com.example.folio",
    );
    request
        .headers_mut()
        .insert("sec-fetch-site", "cross-site".parse().unwrap());
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(stored(&h).await, None);
}

#[tokio::test]
async fn association_sets_no_cookie_for_a_signed_in_visitor() {
    let h = harness_requiring_login().await;
    let admin = vault_session(&h, "admin", folio_core::models::VaultRole::Owner, true).await;
    let app = app_with(&h, &["ABCDE12345.com.example.folio"], true);
    let request = axum::http::Request::get(AASA)
        .header(header::COOKIE, &admin)
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
}
