//! The sidebar shows who is signed in and offers a sign-out.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::{get, harness, harness_requiring_login, text, Harness, SETUP_TOKEN};
use tower::ServiceExt;

fn between<'a>(body: &'a str, start: &str, end: &str) -> &'a str {
    let from = body.find(start).unwrap() + start.len();
    &body[from..from + body[from..].find(end).unwrap()]
}

fn cookie_pair(response: &axum::response::Response, name: &str) -> String {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with(name))
        .map(|c| c.split(';').next().unwrap().to_string())
        .unwrap_or_else(|| panic!("no {name} cookie"))
}

/// Creates the owner account and returns the session cookie
async fn sign_in(h: &Harness) -> String {
    let page = h
        .app
        .clone()
        .oneshot(get(&format!("/setup?token={SETUP_TOKEN}")))
        .await
        .unwrap();
    let csrf_cookie = cookie_pair(&page, "folio_csrf=");
    let csrf = between(&text(page).await, "name=\"csrf\" value=\"", "\"").to_string();
    let form =
        format!("csrf={csrf}&token={SETUP_TOKEN}&username=ada&password=correct-horse-battery");
    let done = h
        .app
        .clone()
        .oneshot(
            Request::post("/setup")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &csrf_cookie)
                .body(Body::from(form))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(done.status(), StatusCode::SEE_OTHER);
    cookie_pair(&done, "folio_session=")
}

#[tokio::test]
async fn signed_in_user_sees_their_name_role_and_a_sign_out_form() {
    let h = harness_requiring_login().await;
    let session = sign_in(&h).await;

    let response = h
        .app
        .clone()
        .oneshot(
            Request::get("/")
                .header(header::COOKIE, &session)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;

    let user = between(&body, "class=\"side-user\"", "</form>");
    assert!(user.contains("ada"), "{user}");
    assert!(user.contains("Owner"), "{user}");
    assert!(user.contains("action=\"/logout\""), "{user}");
    assert!(user.contains("method=\"post\""), "{user}");
    assert!(user.contains("name=\"csrf\""), "{user}");
    assert!(user.contains("Sign out"), "{user}");

    let csrf = between(user, "name=\"csrf\" value=\"", "\"");
    let logout = h
        .app
        .clone()
        .oneshot(
            Request::post("/logout")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &session)
                .body(Body::from(format!("csrf={csrf}")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::SEE_OTHER);
    assert_eq!(logout.headers()[header::LOCATION], "/login");
}

#[tokio::test]
async fn no_user_indicator_when_sign_in_is_not_required() {
    let h = harness().await;
    let response = h.app.clone().oneshot(get("/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = text(response).await;
    assert!(!body.contains("side-user"));
    assert!(!body.contains("action=\"/logout\""));
}
