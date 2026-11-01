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
         UPDATE_OPENAPI=1 cargo test -p folio-api openapi\n\n(or `make openapi`) and commit the result."
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
