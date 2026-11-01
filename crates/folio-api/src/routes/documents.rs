//! Document API routes

use axum::{
    extract::{Multipart, Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, error};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use axum::response::{IntoResponse, Redirect, Response};

use folio_core::models::{
    CreateDocument, Document, DocumentFilter, DocumentSource, SourceKind, UpdateDocument,
};
use folio_core::services::{NewUpload, SearchServiceFacade};

use crate::error::{ApiError, ApiResult};
use crate::openapi::{BadRequest, InternalError, NotFound};
use crate::state::AppState;

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

/// Query parameters for listing documents
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListDocumentsQuery {
    /// Only return documents from this correspondent
    correspondent_id: Option<Uuid>,
    /// Only return documents of this type
    document_type_id: Option<Uuid>,
}

/// Response for document list
#[derive(Debug, Serialize, ToSchema)]
pub struct DocumentListResponse {
    documents: Vec<Document>,
    /// Number of documents returned
    count: usize,
}

/// List documents, optionally filtered by correspondent or document type
#[utoipa::path(
    get,
    path = "/api/documents",
    operation_id = "listDocuments",
    tag = "documents",
    params(ListDocumentsQuery),
    responses(
        (status = 200, description = "Matching documents", body = DocumentListResponse),
        InternalError,
    )
)]
pub(crate) async fn list_documents(
    State(state): State<AppState>,
    Query(query): Query<ListDocumentsQuery>,
) -> ApiResult<Json<DocumentListResponse>> {
    let filter = if query.correspondent_id.is_some() || query.document_type_id.is_some() {
        Some(DocumentFilter {
            correspondent_id: query.correspondent_id,
            document_type_id: query.document_type_id,
            ..Default::default()
        })
    } else {
        None
    };

    let documents = state.document_repo.list(filter).await?;
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
        (status = 200, description = "The document", body = Document),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn get_document(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Document>> {
    let document = state
        .document_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))?;

    Ok(Json(document))
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
    State(state): State<AppState>,
    Json(req): Json<CreateDocumentRequest>,
) -> ApiResult<Json<Document>> {
    let document = state
        .document_repo
        .create(CreateDocument {
            title: req.title,
            content: req.content,
            correspondent_id: req.correspondent_id,
            document_type_id: req.document_type_id,
            source: DocumentSource::of(SourceKind::Api),
            ..Default::default()
        })
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
        (status = 200, description = "The document was deleted"),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn delete_document(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<()>> {
    state.document_repo.delete(id).await?;
    Ok(Json(()))
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
        InternalError,
    )
)]
pub(crate) async fn upload_document(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> ApiResult<Json<Document>> {
    debug!("Starting document upload");

    let mut title: Option<String> = None;
    let mut file_data: Option<Vec<u8>> = None;
    let mut filename: Option<String> = None;
    let mut mime_type: Option<String> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        error!("Multipart error: {}", e);
        ApiError::bad_request(format!("Invalid multipart data: {}", e))
    })? {
        let name = field.name().unwrap_or("").to_string();
        debug!("Processing field: {}", name);

        match name.as_str() {
            "title" => {
                title = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| ApiError::bad_request(format!("Invalid title: {}", e)))?,
                );
            }
            "file" => {
                filename = field.file_name().map(|s| s.to_string());
                mime_type = field.content_type().map(|s| s.to_string());
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::bad_request(format!("Failed to read file: {}", e)))?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }

    let file_data = file_data.ok_or_else(|| ApiError::bad_request("No file provided in upload"))?;
    let title = title
        .or_else(|| filename.clone())
        .unwrap_or_else(|| "Untitled".to_string());
    let mime_type = mime_type.unwrap_or_else(|| "application/octet-stream".to_string());

    let document = state
        .importer
        .import(NewUpload {
            title: Some(title),
            filename: filename.unwrap_or_else(|| "upload".to_string()),
            mime_type: Some(mime_type),
            data: file_data,
            source: DocumentSource::of(SourceKind::Api),
            tag_ids: Vec::new(),
        })
        .await?;

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
    state: &AppState,
    query: &str,
    limit: usize,
) -> ApiResult<Vec<SearchResultWithDocument>> {
    let facade = SearchServiceFacade::new(
        state.document_repo.clone(),
        state.embedding_service.clone(),
        state.search_service.clone(),
    );
    let hits = facade.hybrid_search(query, limit).await?;
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
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> ApiResult<Json<SearchResponse>> {
    let results = semantic_search(&state, &req.query, req.limit).await?;

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
    State(state): State<AppState>,
    Query(params): Query<SearchQueryParams>,
) -> ApiResult<Json<SearchResponse>> {
    let results = semantic_search(&state, &params.q, params.limit).await?;

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
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    let document = state
        .document_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))?;

    let file_path = document
        .file_path
        .ok_or_else(|| ApiError::not_found("Document has no file"))?;

    let storage = folio_infra::FileStorage::new(&state.storage_path);
    let data = storage
        .read(&file_path)
        .await
        .map_err(|_| ApiError::internal("Failed to read file"))?;

    let mime_type = document
        .mime_type
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let filename = document
        .original_filename
        .unwrap_or_else(|| format!("document-{}", id));

    Ok((
        [
            (axum::http::header::CONTENT_TYPE, mime_type),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        data,
    )
        .into_response())
}

/// Form data for setting correspondent
#[derive(Debug, Deserialize)]
pub struct SetCorrespondentForm {
    correspondent_id: Option<String>,
}

/// Set document correspondent (form handler with redirect)
async fn set_correspondent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Form(form): axum::extract::Form<SetCorrespondentForm>,
) -> ApiResult<Response> {
    let correspondent_id = form
        .correspondent_id
        .filter(|s| !s.is_empty())
        .and_then(|s| Uuid::parse_str(&s).ok());

    state
        .document_repo
        .update(
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
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Form(form): axum::extract::Form<SetDocumentTypeForm>,
) -> ApiResult<Response> {
    let document_type_id = form
        .document_type_id
        .filter(|s| !s.is_empty())
        .and_then(|s| Uuid::parse_str(&s).ok());

    state
        .document_repo
        .update(
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
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::extract::Form(form): axum::extract::Form<AddTagForm>,
) -> ApiResult<Response> {
    if let Some(tag_id_str) = form.tag_id.filter(|s| !s.is_empty()) {
        if let Ok(tag_id) = Uuid::parse_str(&tag_id_str) {
            state.document_repo.add_tag(id, tag_id).await?;
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
    State(state): State<AppState>,
    Path(path): Path<RemoveTagPath>,
) -> ApiResult<Response> {
    state.document_repo.remove_tag(path.id, path.tag_id).await?;

    Ok(Redirect::to(&format!("/documents/{}", path.id)).into_response())
}
