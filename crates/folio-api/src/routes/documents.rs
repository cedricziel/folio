//! Document API routes

use axum::{
    body::Body,
    extract::{Multipart, Path, RawQuery, State},
    http::{header, StatusCode},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use folio_core::models::VaultId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use axum::response::{IntoResponse, Redirect, Response};

use folio_core::models::{
    CreateDocument, Document, DocumentFilter, DocumentSource, SourceKind, UpdateDocument,
};
use folio_core::services::SearchServiceFacade;

use super::pages::store_upload;
use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath, ApiQuery};
use crate::openapi::{BadRequest, InternalError, NotFound, PayloadTooLarge};
use crate::state::AppState;
use crate::vault::DefaultVault;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_documents).post(create_document))
        .route("/{id}", get(get_document).delete(delete_document))
        .route("/{id}/download", get(download_document))
        .route("/{id}/correspondent", post(set_correspondent))
        .route("/{id}/document_type", post(set_document_type))
        .route("/{id}/tags", post(add_tag))
        .route("/{id}/tags/{tag_id}", post(remove_tag))
        .route("/upload", post(upload_document))
        .route("/search", get(search_documents_get).post(search_documents))
}

/// Query parameters for listing documents. Only documents the filters all match are returned.
///
/// Parsed by hand in the handler because `tag_id` repeats.
#[derive(Debug, IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code)]
pub struct ListDocumentsQuery {
    /// Only return documents from this correspondent
    correspondent_id: Option<Uuid>,
    /// Only return documents of this type
    document_type_id: Option<Uuid>,
    /// Only return documents that carry every one of these tags; repeat the parameter for several
    #[param(style = Form, explode)]
    tag_id: Option<Vec<Uuid>>,
    /// Only return documents created at or after this instant (RFC 3339)
    created_after: Option<DateTime<Utc>>,
    /// Only return documents created before this instant (RFC 3339)
    created_before: Option<DateTime<Utc>>,
}

impl ListDocumentsQuery {
    fn filter(raw: Option<&str>) -> ApiResult<Option<DocumentFilter>> {
        fn parse<T: std::str::FromStr>(name: &str, value: &str) -> ApiResult<T> {
            value
                .parse()
                .map_err(|_| ApiError::bad_request(format!("Invalid value for {name}")))
        }

        let mut filter = DocumentFilter::default();
        let mut any = false;
        for (key, value) in url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
            any = true;
            match key.as_ref() {
                "correspondent_id" => filter.correspondent_id = Some(parse(&key, &value)?),
                "document_type_id" => filter.document_type_id = Some(parse(&key, &value)?),
                "tag_id" => filter
                    .tag_ids
                    .get_or_insert_with(Vec::new)
                    .push(parse(&key, &value)?),
                "created_after" => filter.created_after = Some(parse(&key, &value)?),
                "created_before" => filter.created_before = Some(parse(&key, &value)?),
                _ => any = false,
            }
        }
        Ok(any.then_some(filter))
    }
}

/// A tag as shown on a document
#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentTag {
    id: Uuid,
    name: String,
    /// Hex color such as "#ff5733", when the tag has one
    color: Option<String>,
}

/// A document together with the names behind its ids
#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentView {
    #[serde(flatten)]
    #[schema(inline = false)]
    document: Document,
    /// Name of the correspondent, when one is set
    correspondent: Option<String>,
    /// Name of the document type, when one is set
    document_type: Option<String>,
    tags: Vec<DocumentTag>,
}

/// Response for document list
#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentListResponse {
    documents: Vec<DocumentView>,
    /// Number of documents returned
    count: usize,
}

async fn present(
    vault: VaultId,
    state: &AppState,
    documents: Vec<Document>,
) -> ApiResult<Vec<DocumentView>> {
    let correspondents: HashMap<_, _> = state
        .correspondent_repo
        .list(vault)
        .await?
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();
    let types: HashMap<_, _> = state
        .document_type_repo
        .list(vault)
        .await?
        .into_iter()
        .map(|t| (t.id, t.name))
        .collect();

    let ids: Vec<_> = documents.iter().map(|document| document.id).collect();
    let mut tags = state.document_repo.tags_for_documents(vault, &ids).await?;
    Ok(documents
        .into_iter()
        .map(|document| {
            let tags = tags
                .remove(&document.id)
                .unwrap_or_default()
                .into_iter()
                .map(|tag| DocumentTag {
                    id: tag.id,
                    name: tag.name,
                    color: tag.color,
                })
                .collect();
            DocumentView {
                correspondent: document
                    .correspondent_id
                    .and_then(|id| correspondents.get(&id).cloned()),
                document_type: document
                    .document_type_id
                    .and_then(|id| types.get(&id).cloned()),
                tags,
                document,
            }
        })
        .collect())
}

/// List documents, newest first, with their tags and the names of their correspondent and type
#[utoipa::path(
    get,
    path = "/api/documents",
    operation_id = "listDocuments",
    tag = "documents",
    params(ListDocumentsQuery),
    responses(
        (status = 200, description = "Matching documents", body = DocumentListResponse),
        BadRequest,
        InternalError,
    )
)]
pub(crate) async fn list_documents(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    RawQuery(query): RawQuery,
) -> ApiResult<Json<DocumentListResponse>> {
    let filter = ListDocumentsQuery::filter(query.as_deref())?;
    let documents = present(
        vault,
        &state,
        state.document_repo.list(vault, filter).await?,
    )
    .await?;
    let count = documents.len();

    Ok(Json(DocumentListResponse { documents, count }))
}

/// Get a single document by ID
#[utoipa::path(
    get,
    path = "/api/documents/{id}",
    operation_id = "getDocument",
    tag = "documents",
    params(("id" = Uuid, Path, description = "Document ID")),
    responses(
        (status = 200, description = "The document", body = DocumentView),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn get_document(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<DocumentView>> {
    let document = state
        .document_repo
        .find_by_id(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))?;
    let view = present(vault, &state, vec![document]).await?.remove(0);

    Ok(Json(view))
}

/// Request body for creating a document
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateDocumentRequest {
    title: String,
    content: Option<String>,
    correspondent_id: Option<Uuid>,
    document_type_id: Option<Uuid>,
}

/// Create a new document without a file
#[utoipa::path(
    post,
    path = "/api/documents",
    operation_id = "createDocument",
    tag = "documents",
    request_body = CreateDocumentRequest,
    responses(
        (status = 200, description = "The created document", body = Document),
        BadRequest,
        InternalError,
    )
)]
pub(crate) async fn create_document(
    DefaultVault(vault): DefaultVault,
    crate::vault::Uploader(uploader): crate::vault::Uploader,
    State(state): State<AppState>,
    ApiJson(req): ApiJson<CreateDocumentRequest>,
) -> ApiResult<Json<Document>> {
    let document = state
        .document_repo
        .create(
            vault,
            CreateDocument {
                title: req.title,
                content: req.content,
                correspondent_id: req.correspondent_id,
                document_type_id: req.document_type_id,
                source: DocumentSource::of(SourceKind::Api),
                uploaded_by: uploader,
                ..Default::default()
            },
        )
        .await?;

    Ok(Json(document))
}

/// Delete a document
#[utoipa::path(
    delete,
    path = "/api/documents/{id}",
    operation_id = "deleteDocument",
    tag = "documents",
    params(("id" = Uuid, Path, description = "Document ID")),
    responses(
        (status = 204, description = "The document was deleted"),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn delete_document(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    purge_document_files(DefaultVault(vault), &state, id).await?;
    state.document_repo.delete(vault, id).await?;
    folio_infra::telemetry::metrics().documents_changed(-1);
    Ok(StatusCode::NO_CONTENT)
}

/// Multipart form accepted by the upload endpoint
#[derive(ToSchema)]
#[allow(dead_code)]
pub struct UploadDocumentRequest {
    /// The file to store and run through OCR
    #[schema(value_type = String, format = Binary)]
    file: Vec<u8>,
    /// Document title; defaults to the file name
    title: Option<String>,
}

/// Upload a document file
#[utoipa::path(
    post,
    path = "/api/documents/upload",
    operation_id = "uploadDocument",
    tag = "documents",
    request_body(content = UploadDocumentRequest, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "The created document", body = Document),
        BadRequest,
        PayloadTooLarge,
        InternalError,
    )
)]
pub(crate) async fn upload_document(
    DefaultVault(vault): DefaultVault,
    crate::vault::Uploader(uploader): crate::vault::Uploader,
    State(state): State<AppState>,
    multipart: Multipart,
) -> ApiResult<Json<Document>> {
    let document = store_upload(&state, vault, uploader, multipart, SourceKind::Api).await?;
    Ok(Json(document))
}

/// Search request body (for POST)
#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchRequest {
    /// Natural-language query
    query: String,
    /// Maximum number of results
    #[serde(default = "default_limit")]
    #[schema(default = 10)]
    limit: usize,
}

/// Search query parameters (for GET)
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQueryParams {
    /// Natural-language query
    q: String,
    /// Maximum number of results
    #[serde(default = "default_limit")]
    #[param(default = 10)]
    limit: usize,
}

fn default_limit() -> usize {
    10
}

/// Search response
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    results: Vec<SearchResultWithDocument>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResultWithDocument {
    document: Document,
    /// Similarity score; higher is closer
    score: f32,
}

async fn semantic_search(
    DefaultVault(vault): DefaultVault,
    state: &AppState,
    query: &str,
    limit: usize,
) -> ApiResult<Vec<SearchResultWithDocument>> {
    let facade = SearchServiceFacade::new(
        state.document_repo.clone(),
        state.embedding_service.clone(),
        state.search_service.clone(),
    );
    let hits = facade.hybrid_search(vault, query, limit).await?;
    Ok(hits
        .into_iter()
        .map(|h| SearchResultWithDocument {
            document: h.document,
            score: h.score,
        })
        .collect())
}

/// Search documents by semantic similarity
#[utoipa::path(
    post,
    path = "/api/documents/search",
    operation_id = "searchDocumentsByBody",
    tag = "documents",
    request_body = SearchRequest,
    responses(
        (status = 200, description = "Documents ranked by similarity", body = SearchResponse),
        InternalError,
    )
)]
pub(crate) async fn search_documents(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiJson(req): ApiJson<SearchRequest>,
) -> ApiResult<Json<SearchResponse>> {
    let results = semantic_search(DefaultVault(vault), &state, &req.query, req.limit).await?;

    Ok(Json(SearchResponse { results }))
}

/// Search documents by semantic similarity using query parameters
#[utoipa::path(
    get,
    path = "/api/documents/search",
    operation_id = "searchDocuments",
    tag = "documents",
    params(SearchQueryParams),
    responses(
        (status = 200, description = "Documents ranked by similarity", body = SearchResponse),
        InternalError,
    )
)]
pub(crate) async fn search_documents_get(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiQuery(params): ApiQuery<SearchQueryParams>,
) -> ApiResult<Json<SearchResponse>> {
    let results = semantic_search(DefaultVault(vault), &state, &params.q, params.limit).await?;

    Ok(Json(SearchResponse { results }))
}

/// Binary payload of a stored file
#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
#[allow(dead_code)]
pub struct FileContent(Vec<u8>);

/// Download the original file of a document
#[utoipa::path(
    get,
    path = "/api/documents/{id}/download",
    operation_id = "downloadDocument",
    tag = "documents",
    params(("id" = Uuid, Path, description = "Document ID")),
    responses(
        (status = 200, description = "The original file; the Content-Type matches the upload",
            body = FileContent, content_type = "application/octet-stream"),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn download_document(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Response> {
    let document = state
        .document_repo
        .find_by_id(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))?;

    let file_path = document
        .file_path
        .ok_or_else(|| ApiError::not_found("Document has no file"))?;

    let object = state.store.get(&file_path).await.map_err(|e| match e {
        folio_core::Error::NotFound { .. } => ApiError::not_found("Stored file not found"),
        e => {
            tracing::error!("Failed to read {file_path}: {e}");
            ApiError::internal("Failed to read file")
        }
    })?;

    let mime_type = document
        .mime_type
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let filename = document
        .original_filename
        .unwrap_or_else(|| format!("document-{}", id));

    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, mime_type)
        .header(header::CONTENT_DISPOSITION, content_disposition(&filename));
    if let Some(size) = object.size {
        response = response.header(header::CONTENT_LENGTH, size);
    }
    let mut response = response
        .body(Body::from_stream(object.stream))
        .map_err(|_| ApiError::internal("Failed to build response"))?;
    super::file_response::harden(&mut response);
    Ok(response)
}

/// `Content-Disposition: attachment` with an ASCII fallback and an RFC 6266
/// `filename*` parameter carrying the exact UTF-8 name.
pub(crate) fn content_disposition(filename: &str) -> String {
    let fallback: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' && c != '%' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for byte in filename.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            )
        {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::content_disposition;

    #[test]
    fn plain_names_get_both_parameters() {
        assert_eq!(
            content_disposition("report.pdf"),
            "attachment; filename=\"report.pdf\"; filename*=UTF-8''report.pdf"
        );
    }

    #[test]
    fn non_ascii_names_are_percent_encoded() {
        let header = content_disposition("Rechnung Größe.pdf");
        assert!(
            header.contains("filename=\"Rechnung Gr__e.pdf\""),
            "{header}"
        );
        assert!(
            header.ends_with("filename*=UTF-8''Rechnung%20Gr%C3%B6%C3%9Fe.pdf"),
            "{header}"
        );
    }

    #[test]
    fn quotes_and_separators_cannot_break_out() {
        let header = content_disposition("a\"; x=\"b\r\n.pdf");
        assert!(!header.contains('\r') && !header.contains('\n'));
        assert_eq!(header.matches('"').count(), 2, "{header}");
    }
}

/// Form data for setting correspondent
#[derive(Debug, Deserialize)]
pub struct SetCorrespondentForm {
    correspondent_id: Option<String>,
}

/// Set document correspondent (form handler with redirect)
async fn set_correspondent(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Form(form): axum::extract::Form<SetCorrespondentForm>,
) -> ApiResult<Response> {
    let correspondent_id =
        super::parse_classification_id(form.correspondent_id, "correspondent_id")?;

    state
        .document_repo
        .update(
            vault,
            id,
            UpdateDocument {
                correspondent_id: Some(correspondent_id),
                ..Default::default()
            },
        )
        .await?;

    Ok(Redirect::to(&format!("/documents/{}", id)).into_response())
}

/// Form data for setting document type
#[derive(Debug, Deserialize)]
pub struct SetDocumentTypeForm {
    document_type_id: Option<String>,
}

/// Set document type (form handler with redirect)
async fn set_document_type(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Form(form): axum::extract::Form<SetDocumentTypeForm>,
) -> ApiResult<Response> {
    let document_type_id =
        super::parse_classification_id(form.document_type_id, "document_type_id")?;

    state
        .document_repo
        .update(
            vault,
            id,
            UpdateDocument {
                document_type_id: Some(document_type_id),
                ..Default::default()
            },
        )
        .await?;

    Ok(Redirect::to(&format!("/documents/{}", id)).into_response())
}

/// Form data for adding a tag
#[derive(Debug, Deserialize)]
pub struct AddTagForm {
    tag_id: Option<String>,
}

/// Add a tag to a document (form handler with redirect)
async fn add_tag(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Form(form): axum::extract::Form<AddTagForm>,
) -> ApiResult<Response> {
    if let Some(tag_id_str) = form.tag_id.filter(|s| !s.is_empty()) {
        if let Ok(tag_id) = Uuid::parse_str(&tag_id_str) {
            state.document_repo.add_tag(vault, id, tag_id).await?;
        }
    }

    Ok(Redirect::to(&format!("/documents/{}", id)).into_response())
}

/// Path parameters for removing a tag
#[derive(Debug, Deserialize)]
pub struct RemoveTagPath {
    id: Uuid,
    tag_id: Uuid,
}

/// Remove a tag from a document (form handler with redirect)
async fn remove_tag(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(path): Path<RemoveTagPath>,
) -> ApiResult<Response> {
    state
        .document_repo
        .remove_tag(vault, path.id, path.tag_id)
        .await?;

    Ok(Redirect::to(&format!("/documents/{}", path.id)).into_response())
}

/// Remove every stored object of a document in the selected vault.
pub(crate) async fn purge_document_files(
    DefaultVault(vault): DefaultVault,
    state: &AppState,
    id: Uuid,
) -> ApiResult<()> {
    if state.document_repo.find_by_id(vault, id).await?.is_none() {
        return Ok(());
    }
    folio_core::ports::purge_document(state.store.as_ref(), id).await?;
    Ok(())
}
