//! OpenAPI description of the JSON API
//!
//! The spec is the contract for generated clients (Swift, Kotlin). Only the
//! JSON, multipart, and SSE endpoints are described; HTML page routes and the
//! form/redirect handlers used by the web UI are deliberately left out.

use axum::{response::Html, routing::get, Json, Router};
use utoipa::{IntoResponses, OpenApi};
use utoipa_scalar::Scalar;

use crate::error::ErrorResponse;
use crate::routes::{correspondents, document_types, documents, events, tags};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Folio API",
        description = "JSON API of the Folio document management system.",
        license(name = "AGPL-3.0-only", identifier = "AGPL-3.0-only"),
    ),
    paths(
        documents::list_documents,
        documents::get_document,
        documents::create_document,
        documents::delete_document,
        documents::upload_document,
        documents::download_document,
        documents::search_documents_get,
        documents::search_documents,
        tags::list_tags,
        tags::get_tag,
        tags::create_tag,
        tags::update_tag,
        tags::delete_tag,
        correspondents::list_correspondents,
        correspondents::get_correspondent,
        correspondents::create_correspondent,
        correspondents::update_correspondent,
        correspondents::delete_correspondent,
        document_types::list_document_types,
        document_types::get_document_type,
        document_types::create_document_type,
        document_types::update_document_type,
        document_types::delete_document_type,
        events::sse_handler,
    ),
    components(schemas(ErrorResponse)),
    tags(
        (name = "documents", description = "Documents and their files"),
        (name = "tags", description = "Tags for categorizing documents"),
        (name = "correspondents", description = "Senders of documents"),
        (name = "document_types", description = "Document classifications"),
        (name = "events", description = "Real-time updates"),
    )
)]
struct ApiDoc;

/// Shared error responses referenced from the `#[utoipa::path]` annotations.
macro_rules! error_response {
    ($name:ident, $status:literal, $description:literal) => {
        #[derive(IntoResponses)]
        #[response(status = $status, description = $description)]
        #[allow(dead_code)]
        pub(crate) struct $name(ErrorResponse);
    };
}

error_response!(
    BadRequest,
    400,
    "The request is malformed or fails validation"
);
error_response!(NotFound, 404, "The resource does not exist");
error_response!(
    Conflict,
    409,
    "A resource with the same unique value already exists"
);
error_response!(InternalError, 500, "Unexpected server error");

/// The OpenAPI document describing the JSON API.
pub fn openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

/// The spec as pretty-printed JSON, exactly as committed to `openapi/openapi.json`.
pub fn spec_json() -> String {
    let mut json = openapi()
        .to_pretty_json()
        .expect("OpenAPI document serializes to JSON");
    json.push('\n');
    json
}

/// Routes serving the spec (`/api/openapi.json`) and an interactive viewer (`/api/docs`).
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/openapi.json", get(|| async { Json(openapi()) }))
        .route(
            "/api/docs",
            get(|| async { Html(Scalar::new(openapi()).to_html()) }),
        )
}
