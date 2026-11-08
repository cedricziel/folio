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
