mod common;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    middleware,
    routing::post,
    Json, Router,
};
use folio_api::{enforce_auth, AuthState};
use folio_auth::{crypto::hash_token, AccessToken, OAuthClient, Session, User};
use folio_core::{
    models::UserId,
    mutation::{current_actor, with_actor},
};
use tower::ServiceExt;

async fn actor(Json(_body): Json<serde_json::Value>) -> Json<Option<UserId>> {
    tokio::task::yield_now().await;
    Json(current_actor())
}

async fn fixture(required: bool) -> (common::Harness, Router) {
    let h = common::harness().await;
    h.auth_store
        .create_user(User {
            id: "alice".into(),
            username: "alice".into(),
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
            id_hash: hash_token("session"),
            user_id: "alice".into(),
            expires_at: i64::MAX / 2,
            created_at: 1,
            user_agent: None,
        })
        .await
        .unwrap();
    h.auth_store
        .insert_client(OAuthClient {
            client_id: "actor-client".into(),
            client_name: "actor test".into(),
            redirect_uris: vec!["http://localhost/callback".into()],
            created_at: 1,
        })
        .await
        .unwrap();
    h.auth_store
        .insert_access_token(AccessToken {
            token_hash: hash_token("bearer"),
            client_id: "actor-client".into(),
            user_id: "alice".into(),
            scopes: vec!["documents:write".into()],
            audience: "http://localhost/api".into(),
            expires_at: i64::MAX / 2,
            grant_id: "actor-grant".into(),
        })
        .await
        .unwrap();
    let app = Router::new()
        .route("/api/actor", post(actor))
        .route("/api/tags", post(actor))
        .route("/login", post(actor))
        .layer(middleware::from_fn_with_state(
            AuthState::new(h.auth_store.clone(), None).with_required(required),
            enforce_auth,
        ));
    (h, app)
}

fn request(path: &str) -> axum::http::request::Builder {
    Request::post(path).header(header::CONTENT_TYPE, "application/json")
}

#[tokio::test]
async fn session_actor_comes_from_authentication_instead_of_json() {
    let (_h, app) = fixture(true).await;
    let response = app
        .oneshot(
            request("/api/actor")
                .header(header::COOKIE, "folio_session=session")
                .body(Body::from(r#"{"actor":"forged","created_by":"forged"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(common::text(response).await, r#""alice""#);
    assert_eq!(current_actor(), None);
}

#[tokio::test]
async fn bearer_actor_comes_from_the_validated_token() {
    let (_h, app) = fixture(true).await;
    let response = app
        .oneshot(
            request("/api/actor")
                .header(header::AUTHORIZATION, "Bearer bearer")
                .body(Body::from(r#"{"actor":"forged"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(common::text(response).await, r#""alice""#);
}

#[tokio::test]
async fn unauthenticated_requests_reset_an_enclosing_actor_when_auth_is_optional() {
    let (_h, app) = fixture(false).await;
    let response = with_actor(
        Some(UserId::from("enclosing")),
        app.oneshot(request("/api/actor").body(Body::from("{}")).unwrap()),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(common::text(response).await, "null");
}

#[tokio::test]
async fn exempt_requests_reset_an_enclosing_actor() {
    let (_h, app) = fixture(true).await;
    let response = with_actor(
        Some(UserId::from("enclosing")),
        app.oneshot(
            request("/login")
                .header(header::COOKIE, "folio_session=session")
                .body(Body::from("{}"))
                .unwrap(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(common::text(response).await, "null");
}

#[tokio::test]
async fn unverified_requests_do_not_reach_mutations() {
    let (_h, app) = fixture(true).await;
    let response = app
        .oneshot(
            request("/api/actor")
                .extension(folio_auth::AuthContext {
                    personal_token_id: None,
                    user_id: "alice".into(),
                    client_id: "forged".into(),
                    scopes: vec!["documents:write".into()],
                    audience: "http://localhost/api".into(),
                    grant_id: String::new(),
                })
                .header("x-folio-actor", "alice")
                .body(Body::from(r#"{"actor":"alice"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn scoped_personal_token_actor_comes_from_the_token_owner() {
    let (h, app) = fixture(true).await;
    h.auth_store
        .insert_personal_token(folio_auth::PersonalToken {
            id: "actor-pat".into(),
            user_id: "alice".into(),
            name: "actor test".into(),
            token_hash: hash_token("folio_pat_actor"),
            prefix: "folio_pat_actor".into(),
            audience: "http://localhost/api".into(),
            scopes: vec!["documents:write".into()],
            created_at: 1,
            expires_at: i64::MAX / 2,
            revoked_at: None,
        })
        .await
        .unwrap();
    let response = app
        .oneshot(
            request("/api/tags")
                .header(header::AUTHORIZATION, "Bearer folio_pat_actor")
                .body(Body::from(r#"{"actor":"forged"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(common::text(response).await, r#""alice""#);
}

#[tokio::test]
async fn optional_authentication_still_attributes_valid_sessions() {
    let (_h, app) = fixture(false).await;
    let response = app
        .oneshot(
            request("/api/actor")
                .header(header::COOKIE, "folio_session=session")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(common::text(response).await, r#""alice""#);
}
