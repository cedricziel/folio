//! HTML error pages for browsers
//!
//! Handlers fail with JSON (`ApiError`) and unknown routes with an empty 404.
//! People opening a page in a browser should get the app shell instead, so
//! this layer swaps those bodies for a rendered page when the request asks for HTML.

use askama::Template;
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Response},
};

use crate::state::AppState;

use super::view::{Nav, Sidebar};

/// Paths that speak JSON or other protocols, whatever the client accepts
const MACHINE_PREFIXES: [&str; 5] = ["/api", "/mcp", "/oauth", "/.well-known", "/static"];

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorTemplate {
    sidebar: Sidebar,
    heading: &'static str,
    message: &'static str,
}

fn wording(status: StatusCode) -> Option<(&'static str, &'static str)> {
    match status {
        StatusCode::NOT_FOUND => Some((
            "Not found",
            "This page isn't here. The document may have been deleted, or the link is out of date.",
        )),
        s if s.is_server_error() => Some((
            "Something went wrong",
            "Something went wrong on this server. Try again, or check the server log.",
        )),
        _ => None,
    }
}

fn wants_html(request: &Request) -> bool {
    let accepts_html = request
        .headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html"));
    let path = request.uri().path();
    accepts_html
        && !MACHINE_PREFIXES
            .iter()
            .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

fn is_html(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"))
}

/// Replaces 404 and 5xx answers that are not already HTML with a page in the app shell
pub async fn html_errors(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let wants_html = wants_html(&request);
    let response = next.run(request).await;
    if !wants_html || is_html(&response) {
        return response;
    }
    let Some((heading, message)) = wording(response.status()) else {
        return response;
    };
    let Ok(sidebar) = Sidebar::load(&state, Nav::None).await else {
        return response;
    };
    match (ErrorTemplate {
        sidebar,
        heading,
        message,
    })
    .render()
    {
        Ok(body) => (response.status(), Html(body)).into_response(),
        Err(_) => response,
    }
}
