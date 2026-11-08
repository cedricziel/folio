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
