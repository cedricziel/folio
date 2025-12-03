//! Document API routes

use axum::{
    extract::{Multipart, Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, error};
use uuid::Uuid;

use axum::response::{IntoResponse, Redirect, Response};

use folio_core::models::{CreateDocument, Document, DocumentFilter, UpdateDocument};

use crate::error::{ApiError, ApiResult};
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
#[derive(Debug, Deserialize)]
pub struct ListDocumentsQuery {
    correspondent_id: Option<Uuid>,
    document_type_id: Option<Uuid>,
}

/// Response for document list
#[derive(Debug, Serialize)]
pub struct DocumentListResponse {
    documents: Vec<Document>,
    count: usize,
}

/// List all documents
async fn list_documents(
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
async fn get_document(
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
#[derive(Debug, Deserialize)]
pub struct CreateDocumentRequest {
    title: String,
    content: Option<String>,
    correspondent_id: Option<Uuid>,
    document_type_id: Option<Uuid>,
}

/// Create a new document
async fn create_document(
    State(state): State<AppState>,
    Json(req): Json<CreateDocumentRequest>,
) -> ApiResult<Json<Document>> {
    let document = state
        .document_repo
        .create(CreateDocument {
            title: req.title,
            content: req.content,
            original_filename: None,
            mime_type: None,
            correspondent_id: req.correspondent_id,
            document_type_id: req.document_type_id,
        })
        .await?;

    Ok(Json(document))
}

/// Delete a document
async fn delete_document(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<()>> {
    state.document_repo.delete(id).await?;
    Ok(Json(()))
}

/// Upload a document with file
async fn upload_document(
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

    // Extract text from document using OCR
    let content = state
        .ocr_service
        .extract_text(&file_data, &mime_type)
        .await
        .ok();

    // Create document record
    let document = state
        .document_repo
        .create(CreateDocument {
            title,
            content: content.clone(),
            original_filename: filename.clone(),
            mime_type: Some(mime_type),
            correspondent_id: None,
            document_type_id: None,
        })
        .await?;

    // Store the file
    let storage = folio_infra::FileStorage::new(&state.storage_path);
    if let Some(fname) = filename {
        if let Ok(file_path) = storage.store(document.id, &fname, &file_data).await {
            let _ = state
                .document_repo
                .set_file_path(document.id, &file_path)
                .await;
        }
    }

    // Generate and store embedding for search
    if let Some(ref text) = content {
        if !text.is_empty() {
            if let Ok(embedding) = state.embedding_service.generate_embedding(text).await {
                let _ = state
                    .search_service
                    .store_embedding(document.id, &embedding)
                    .await;
            }
        }
    }

    // Re-fetch to get updated document with file_path
    let document = state
        .document_repo
        .find_by_id(document.id)
        .await?
        .unwrap_or(document);

    Ok(Json(document))
}

/// Search request body (for POST)
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

/// Search query parameters (for GET)
#[derive(Debug, Deserialize)]
pub struct SearchQueryParams {
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    10
}

/// Search response
#[derive(Debug, Serialize)]
pub struct SearchResponse {
    results: Vec<SearchResultWithDocument>,
}

#[derive(Debug, Serialize)]
pub struct SearchResultWithDocument {
    document: Document,
    score: f32,
}

/// Search documents by semantic similarity
async fn search_documents(
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> ApiResult<Json<SearchResponse>> {
    // Generate embedding for query
    let query_embedding = state
        .embedding_service
        .generate_embedding(&req.query)
        .await?;

    // Search for similar documents
    let search_results = state
        .search_service
        .search(&query_embedding, req.limit)
        .await?;

    // Fetch full document details
    let mut results = Vec::new();
    for result in search_results {
        if let Ok(Some(document)) = state.document_repo.find_by_id(result.document_id).await {
            results.push(SearchResultWithDocument {
                document,
                score: result.score,
            });
        }
    }

    Ok(Json(SearchResponse { results }))
}

/// Search documents by semantic similarity (GET handler for command palette)
async fn search_documents_get(
    State(state): State<AppState>,
    Query(params): Query<SearchQueryParams>,
) -> ApiResult<Json<SearchResponse>> {
    // Generate embedding for query
    let query_embedding = state
        .embedding_service
        .generate_embedding(&params.q)
        .await?;

    // Search for similar documents
    let search_results = state
        .search_service
        .search(&query_embedding, params.limit)
        .await?;

    // Fetch full document details
    let mut results = Vec::new();
    for result in search_results {
        if let Ok(Some(document)) = state.document_repo.find_by_id(result.document_id).await {
            results.push(SearchResultWithDocument {
                document,
                score: result.score,
            });
        }
    }

    Ok(Json(SearchResponse { results }))
}

/// Download a document file
async fn download_document(
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
