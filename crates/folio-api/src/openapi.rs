//! OpenAPI description of the JSON API
//!
//! The spec is the contract for generated clients (Swift, Kotlin). Only the
//! JSON, multipart, and SSE endpoints are described; HTML page routes and the
//! form/redirect handlers used by the web UI are deliberately left out.

use std::sync::LazyLock;

use axum::{http::header, response::Html, routing::get, Router};
use utoipa::{IntoResponses, OpenApi};

use folio_core::models::WebhookPayload;

use crate::client_telemetry;
use crate::error::ErrorResponse;
use crate::routes::{
    correspondents, document_entries, document_types, documents, events, sources, tags, tokens,
    vaults, workflows,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Folio API",
        description = "JSON API of the Folio document management system.",
        license(name = "AGPL-3.0-only", identifier = "AGPL-3.0-only"),
    ),
    paths(
        tokens::list_personal_tokens,
        tokens::create_personal_token,
        tokens::revoke_personal_token,
        client_telemetry::configuration,
        client_telemetry::export,
        vaults::list_vaults,
        crate::routes::saved_views::list,
        crate::routes::saved_views::find,
        crate::routes::saved_views::create,
        crate::routes::saved_views::update,
        crate::routes::saved_views::delete,
        documents::list_documents,
        crate::routes::browse::browse_default,
        crate::routes::browse::browse_vault,
        documents::get_document,
        documents::update_document_date,
        document_entries::list,
        document_entries::create,
        document_entries::update,
        document_entries::delete,
        documents::create_document,
        documents::delete_document,
        crate::routes::trash::list,
        crate::routes::trash::restore,
        crate::routes::trash::purge,
        documents::upload_document,
        documents::download_document,
        documents::document_thumbnail,
        documents::search_documents_get,
        documents::search_documents,
        tags::list_tags,
        tags::list_tags_trash,
        tags::restore_tag,
        tags::get_tag,
        tags::create_tag,
        tags::update_tag,
        tags::delete_tag,
        correspondents::list_correspondents,
        correspondents::list_correspondents_trash,
        correspondents::restore_correspondent,
        correspondents::get_correspondent,
        correspondents::create_correspondent,
        correspondents::update_correspondent,
        correspondents::delete_correspondent,
        document_types::list_document_types,
        document_types::list_document_types_trash,
        document_types::restore_document_type,
        document_types::get_document_type,
        document_types::create_document_type,
        document_types::update_document_type,
        document_types::delete_document_type,
        events::sse_handler,
        workflows::list_workflows,
        workflows::get_workflow,
        workflows::create_workflow,
        workflows::update_workflow,
        workflows::delete_workflow,
        workflows::test_workflow,
        workflows::list_runs,
        sources::list_sources,
        sources::get_source,
        sources::create_source,
        sources::update_source,
        sources::delete_source,
        sources::test_source,
    ),
    components(schemas(ErrorResponse, WebhookPayload)),
    tags(
        (name = "authentication", description = "Session-only personal access token management"),
        (name = "telemetry", description = "Authenticated client OTLP export"),
        (name = "documents", description = "Documents and their files"),
        (name = "tags", description = "Tags for categorizing documents"),
        (name = "correspondents", description = "Senders of documents"),
        (name = "document_types", description = "Document classifications"),
        (name = "events", description = "Real-time updates"),
        (name = "workflows", description = "Automations that react to document events, such as webhooks"),
        (name = "sources", description = "Consume folders and mailboxes that import documents automatically"),
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
error_response!(PayloadTooLarge, 413, "The upload exceeds the 50 MB limit");
error_response!(InternalError, 500, "Unexpected server error");

/// The OpenAPI document describing the JSON API.
pub fn openapi() -> utoipa::openapi::OpenApi {
    use utoipa::openapi::{
        path::{ParameterBuilder, ParameterIn},
        schema::{ObjectBuilder, Type},
        security::{
            ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityRequirement, SecurityScheme,
        },
    };
    let mut spec = ApiDoc::openapi();
    let components = spec.components.get_or_insert_with(Default::default);
    components.add_security_scheme("bearerAuth", SecurityScheme::Http(
        HttpBuilder::new().scheme(HttpAuthScheme::Bearer)
            .bearer_format("opaque")
            .description(Some("OAuth access token or personal access token for this server's /api audience. Personal tokens support document, vocabulary, and vault-discovery endpoints only. Personal tokens use explicit documents:read and documents:write scopes; write does not imply read. Current vault permissions also apply. MCP requires a separate /mcp audience token. Explicit bearer credentials take precedence over session cookies."))
            .build(),
    ));
    components.add_security_scheme("sessionAuth", SecurityScheme::ApiKey(ApiKey::Cookie(
        ApiKeyValue::with_description("__Host-folio_session", "Signed-in browser session. Local HTTP deployments use folio_session. Session mutations require same-origin protection. Personal token management requires a session and rejects bearer credentials."),
    )));
    for (path, item) in &mut spec.paths.paths {
        for operation in [
            &mut item.get,
            &mut item.post,
            &mut item.put,
            &mut item.patch,
            &mut item.delete,
            &mut item.head,
        ]
        .into_iter()
        .flatten()
        {
            let mut requirements = vec![SecurityRequirement::new(
                "sessionAuth",
                Vec::<String>::new(),
            )];
            if path.starts_with("/api/") && !path.starts_with("/api/auth/tokens") {
                requirements.push(SecurityRequirement::new("bearerAuth", Vec::<String>::new()));
            }
            operation.security = Some(requirements);
            operation.responses.responses.entry("401".to_string()).or_insert_with(||
                utoipa::openapi::Response::new("Authentication is missing or the credential is invalid, expired, revoked, or issued for another audience").into());
            operation
                .responses
                .responses
                .entry("403".to_string())
                .or_insert_with(|| {
                    utoipa::openapi::Response::new(
                        "The credential lacks the required scope or the request is not permitted",
                    )
                    .into()
                });
        }
    }
    for (path, item) in &mut spec.paths.paths {
        if !crate::vault_context::is_scoped_path(path) {
            continue;
        }
        for operation in [
            &mut item.get,
            &mut item.post,
            &mut item.put,
            &mut item.patch,
            &mut item.delete,
            &mut item.head,
        ]
        .into_iter()
        .flatten()
        {
            operation.responses.responses.insert(
                "403".to_string(),
                utoipa::openapi::Response::new(
                    "The token scope or vault role does not allow this action",
                )
                .into(),
            );
            operation
                .responses
                .responses
                .entry("404".to_string())
                .or_insert_with(|| {
                    utoipa::openapi::Response::new("The vault or resource is not visible").into()
                });
            operation.parameters.get_or_insert_with(Vec::new).push(ParameterBuilder::new()
                .name("X-Folio-Vault")
                .parameter_in(ParameterIn::Header)
                .description(Some("Optional vault UUID. Omit to use the account default. Unknown or inaccessible vaults return 404; disallowed actions return 403."))
                .schema(Some(ObjectBuilder::new().schema_type(Type::String)))
                .build().into());
        }
    }
    spec
}

/// The spec as pretty-printed JSON, built once: the document never changes while running.
static SPEC_JSON: LazyLock<String> = LazyLock::new(|| {
    let mut json = openapi()
        .to_pretty_json()
        .expect("OpenAPI document serializes to JSON");
    json.push('\n');
    json
});

/// Viewer page. Scalar is vendored under `static/vendor`, so the page works offline.
const DOCS_HTML: &str = r#"<!doctype html>
<html>
<head>
    <title>Folio API</title>
    <meta charset="utf-8"/>
    <meta name="viewport" content="width=device-width, initial-scale=1"/>
</head>
<body>
<script id="api-reference" data-url="/api/openapi.json"></script>
<script src="/static/vendor/scalar.js"></script>
</body>
</html>
"#;

/// The spec as pretty-printed JSON, exactly as committed to `openapi/openapi.json`.
pub fn spec_json() -> String {
    SPEC_JSON.clone()
}

/// Routes serving the spec (`/api/openapi.json`) and an interactive viewer (`/api/docs`).
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/api/openapi.json",
            get(|| async { ([(header::CONTENT_TYPE, "application/json")], &**SPEC_JSON) }),
        )
        .route("/api/docs", get(|| async { Html(DOCS_HTML) }))
}
