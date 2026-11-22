//! The OpenAPI spec is the contract for generated clients, so the committed
//! copy must track the code.

use std::{collections::HashSet, path::PathBuf};

use axum::{body::Body, http::Request};
use folio_api::openapi;
use tower::ServiceExt;

fn committed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../openapi/openapi.json")
}

#[test]
fn committed_spec_matches_generated_spec() {
    let generated = openapi::spec_json();
    let path = committed_path();

    if std::env::var_os("UPDATE_OPENAPI").is_some() {
        std::fs::write(&path, &generated).expect("write openapi/openapi.json");
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "openapi/openapi.json is out of date. Regenerate it with:\n\n    \
         UPDATE_OPENAPI=1 cargo test -p folio-api --test openapi\n\n(or `make openapi`) and commit the result."
    );
}

#[tokio::test]
async fn serves_spec_as_json() {
    let response = openapi::router::<()>()
        .oneshot(
            Request::get("/api/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
    assert!(json["openapi"].as_str().unwrap().starts_with("3.1"));
    assert!(json["paths"]["/api/documents"]["get"].is_object());
}

#[test]
fn operation_ids_are_unique_camel_case() {
    let spec: serde_json::Value = serde_json::from_str(&openapi::spec_json()).unwrap();
    let mut seen = HashSet::new();

    for (path, item) in spec["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            let id = op["operationId"]
                .as_str()
                .unwrap_or_else(|| panic!("{method} {path} has no operationId"));
            assert!(
                id.starts_with(|c: char| c.is_ascii_lowercase())
                    && id.chars().all(|c| c.is_ascii_alphanumeric()),
                "{method} {path}: operationId `{id}` is not camelCase"
            );
            assert!(seen.insert(id.to_string()), "duplicate operationId `{id}`");
        }
    }
}

#[tokio::test]
async fn docs_page_loads_the_vendored_viewer_without_a_cdn() {
    let response = openapi::router::<()>()
        .oneshot(Request::get("/api/docs").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains("/static/vendor/scalar.js"));
    assert!(!html.contains("https://"), "{html}");
}

#[test]
fn documents_describe_bearer_auth_and_token_management_requires_session() {
    let spec: serde_json::Value = serde_json::from_str(&openapi::spec_json()).unwrap();
    assert_eq!(
        spec["components"]["securitySchemes"]["bearerAuth"]["scheme"],
        "bearer"
    );
    assert_eq!(
        spec["components"]["securitySchemes"]["sessionAuth"]["in"],
        "cookie"
    );
    assert_eq!(
        spec["paths"]["/api/documents"]["get"]["security"],
        serde_json::json!([
            {"sessionAuth": []}, {"bearerAuth": []}
        ])
    );
    assert_eq!(
        spec["paths"]["/api/auth/tokens"]["post"]["security"],
        serde_json::json!([
            {"sessionAuth": []}
        ])
    );
    assert!(spec["paths"]["/api/auth/tokens"]["post"]["responses"]["201"].is_object());
    assert!(spec["paths"]["/api/auth/tokens/{id}"]["delete"]["responses"]["404"].is_object());
}

#[test]
fn review_metadata_uses_nullable_inline_types_for_generated_clients() {
    let spec: serde_json::Value = serde_json::from_str(&openapi::spec_json()).unwrap();
    let fields = &spec["components"]["schemas"]["Document"]["properties"];
    assert_eq!(
        fields["reviewed_at"]["type"],
        serde_json::json!(["string", "null"])
    );
    assert_eq!(fields["reviewed_at"]["format"], "date-time");
    assert_eq!(
        fields["reviewed_by"]["type"],
        serde_json::json!(["string", "null"])
    );
    assert_eq!(
        fields["review_origin"]["type"],
        serde_json::json!(["string", "null"])
    );
}
