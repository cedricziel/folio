//! Page routes for Turbo-based web UI

use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::header,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use tracing::{debug, error};
use uuid::Uuid;

use folio_core::models::{
    Correspondent, CreateDocument, Document, DocumentType, Tag, UpdateDocument,
};
use folio_infra::JobType;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(index_page))
        .route("/documents", get(documents_list))
        // Static routes MUST come before parameterized routes
        .route("/documents/upload", post(upload_document))
        .route("/documents/search", get(search_documents))
        // Parameterized routes come last
        .route("/documents/:id", get(document_detail))
        .route("/documents/:id/thumbnail", get(document_thumbnail))
        .route(
            "/documents/:id/page/:page/thumbnail",
            get(document_page_thumbnail),
        )
        .route("/documents/:id/delete", post(delete_document))
        .route(
            "/documents/:id/correspondent",
            post(set_document_correspondent),
        )
        .route("/documents/:id/document_type", post(set_document_type))
        .route("/documents/:id/tags", post(add_document_tag))
        .route("/documents/:id/tags/:tag_id", post(remove_document_tag))
        .route("/documents/:id/reprocess", post(reprocess_document))
}

/// Index page template
#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    title: String,
    documents: Vec<Document>,
    correspondents: Vec<Correspondent>,
    document_types: Vec<DocumentType>,
    tags: Vec<Tag>,
    filter_correspondent_id: Option<String>,
    filter_document_type_id: Option<String>,
    filter_tag_id: Option<String>,
}

/// Query parameters for filtering documents
#[derive(Debug, serde::Deserialize, Default)]
pub struct FilterQuery {
    correspondent_id: Option<String>,
    document_type_id: Option<String>,
    tag_id: Option<String>,
}

/// Main index page
async fn index_page(
    State(state): State<AppState>,
    Query(filter_query): Query<FilterQuery>,
) -> ApiResult<Html<String>> {
    use folio_core::models::DocumentFilter;

    // Parse UUIDs from strings, treating empty strings as None
    let correspondent_id = filter_query
        .correspondent_id
        .as_ref()
        .filter(|s| !s.is_empty())
        .and_then(|s| Uuid::parse_str(s).ok());
    let document_type_id = filter_query
        .document_type_id
        .as_ref()
        .filter(|s| !s.is_empty())
        .and_then(|s| Uuid::parse_str(s).ok());
    let tag_id = filter_query
        .tag_id
        .as_ref()
        .filter(|s| !s.is_empty())
        .and_then(|s| Uuid::parse_str(s).ok());

    let filter = if correspondent_id.is_some() || document_type_id.is_some() || tag_id.is_some() {
        Some(DocumentFilter {
            correspondent_id,
            document_type_id,
            tag_ids: tag_id.map(|id| vec![id]),
            ..Default::default()
        })
    } else {
        None
    };

    let documents = state.document_repo.list(filter).await?;
    let correspondents = state.correspondent_repo.list().await?;
    let document_types = state.document_type_repo.list().await?;
    let tags = state.tag_repo.list().await?;

    let template = IndexTemplate {
        title: "Folio - Document Management".to_string(),
        documents,
        correspondents,
        document_types,
        tags,
        filter_correspondent_id: correspondent_id.map(|id| id.to_string()),
        filter_document_type_id: document_type_id.map(|id| id.to_string()),
        filter_tag_id: tag_id.map(|id| id.to_string()),
    };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering page: {}", e)
    })))
}

/// Documents list template (Turbo Frame partial)
#[derive(Template)]
#[template(path = "partials/documents_frame.html")]
struct DocumentsFrameTemplate {
    documents: Vec<Document>,
}

/// Documents list (for Turbo Frame)
async fn documents_list(State(state): State<AppState>) -> ApiResult<Html<String>> {
    let documents = state.document_repo.list(None).await?;

    let template = DocumentsFrameTemplate { documents };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering list: {}", e)
    })))
}

/// Document view template (full page)
#[derive(Template)]
#[template(path = "document_view.html")]
struct DocumentViewTemplate {
    document: Document,
    correspondent: Option<Correspondent>,
    document_type: Option<DocumentType>,
    tags: Vec<Tag>,
    all_correspondents: Vec<Correspondent>,
    all_document_types: Vec<DocumentType>,
    all_tags: Vec<Tag>,
    show_page_nav: bool,
}

/// Document detail (full page view)
async fn document_detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Html<String>> {
    let document = state
        .document_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| crate::error::ApiError::not_found("Document not found"))?;

    // Fetch related entities
    let correspondent = if let Some(correspondent_id) = document.correspondent_id {
        state
            .correspondent_repo
            .find_by_id(correspondent_id)
            .await?
    } else {
        None
    };

    let document_type = if let Some(document_type_id) = document.document_type_id {
        state
            .document_type_repo
            .find_by_id(document_type_id)
            .await?
    } else {
        None
    };

    let tags = state.document_repo.get_tags(document.id).await?;

    // Fetch all options for editing
    let all_correspondents = state.correspondent_repo.list().await?;
    let all_document_types = state.document_type_repo.list().await?;
    let all_tags = state.tag_repo.list().await?;

    let show_page_nav = document.page_count.map(|c| c > 1).unwrap_or(false);

    let template = DocumentViewTemplate {
        document,
        correspondent,
        document_type,
        tags,
        all_correspondents,
        all_document_types,
        all_tags,
        show_page_nav,
    };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering document view: {}", e)
    })))
}

/// Document thumbnail (serves generated thumbnail or placeholder)
async fn document_thumbnail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    let document = state
        .document_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))?;

    // Try to serve generated thumbnail first
    if state.thumbnail_service.exists(id).await {
        if let Ok(data) = state.thumbnail_service.read(id).await {
            return Ok(([(header::CONTENT_TYPE, "image/jpeg")], data).into_response());
        }
    }

    // Fallback: try to serve the original image if it's an image
    let is_image = document
        .mime_type
        .as_ref()
        .map(|m| m.starts_with("image/"))
        .unwrap_or(false);

    if is_image {
        if let Some(ref file_path) = document.file_path {
            let storage = folio_infra::FileStorage::new(&state.storage_path);
            if let Ok(data) = storage.read(file_path).await {
                let mime = document
                    .mime_type
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                return Ok(([(header::CONTENT_TYPE, mime)], data).into_response());
            }
        }
    }

    // Return a placeholder SVG for non-images or missing files
    let is_pdf = document
        .mime_type
        .as_ref()
        .map(|m| m == "application/pdf")
        .unwrap_or(false);

    let placeholder = if is_pdf {
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="100" height="100">
            <rect width="100" height="100" fill="#f1f5f9"/>
            <path d="M25 10h35l15 15v60a5 5 0 01-5 5H25a5 5 0 01-5-5V15a5 5 0 015-5z" fill="#ef4444"/>
            <path d="M60 10v15h15" fill="#dc2626"/>
            <text x="50" y="60" text-anchor="middle" fill="white" font-family="sans-serif" font-size="14" font-weight="bold">PDF</text>
        </svg>"##
    } else {
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="100" height="100">
            <rect width="100" height="100" fill="#f1f5f9"/>
            <path d="M25 10h35l15 15v60a5 5 0 01-5 5H25a5 5 0 01-5-5V15a5 5 0 015-5z" fill="#64748b"/>
            <path d="M60 10v15h15" fill="#475569"/>
            <text x="50" y="60" text-anchor="middle" fill="white" font-family="sans-serif" font-size="12" font-weight="bold">FILE</text>
        </svg>"##
    };

    Ok(([(header::CONTENT_TYPE, "image/svg+xml")], placeholder).into_response())
}

/// Document page thumbnail (serves generated thumbnail for a specific page)
async fn document_page_thumbnail(
    State(state): State<AppState>,
    Path((id, page)): Path<(Uuid, u32)>,
) -> ApiResult<Response> {
    let document = state
        .document_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))?;

    // Validate page number
    if page == 0 {
        return Err(ApiError::bad_request("Page number must be at least 1"));
    }

    if let Some(page_count) = document.page_count {
        if page > page_count {
            return Err(ApiError::not_found("Page not found"));
        }
    }

    // Try to serve generated page thumbnail
    if state.thumbnail_service.page_exists(id, page).await {
        if let Ok(data) = state.thumbnail_service.read_page(id, page).await {
            return Ok(([(header::CONTENT_TYPE, "image/jpeg")], data).into_response());
        }
    }

    // Fallback: return a placeholder SVG for the page
    let placeholder = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 140" width="100" height="140">
            <rect width="100" height="140" fill="#f1f5f9"/>
            <path d="M15 10h50l15 15v100a5 5 0 01-5 5H15a5 5 0 01-5-5V15a5 5 0 015-5z" fill="#94a3b8"/>
            <path d="M65 10v15h15" fill="#64748b"/>
            <text x="50" y="80" text-anchor="middle" fill="white" font-family="sans-serif" font-size="16" font-weight="bold">{}</text>
        </svg>"##,
        page
    );

    Ok(([(header::CONTENT_TYPE, "image/svg+xml")], placeholder).into_response())
}

/// Upload success template (Turbo Stream response)
#[derive(Template)]
#[template(path = "partials/upload_success.html")]
struct UploadSuccessTemplate {
    document: Document,
    documents: Vec<Document>,
}

/// Upload a document (returns Turbo Stream)
async fn upload_document(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> ApiResult<Response> {
    debug!("Starting document upload (HTML)");

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
                let text = field
                    .text()
                    .await
                    .map_err(|e| ApiError::bad_request(format!("Invalid title: {}", e)))?;
                if !text.is_empty() {
                    title = Some(text);
                }
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

    // Create document record (OCR will be done by background worker)
    let document = state
        .document_repo
        .create(CreateDocument {
            title,
            content: None,
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

    // Enqueue background job for thumbnail, OCR, and embedding
    if let Err(e) = state
        .job_queue
        .enqueue(JobType::ProcessDocument {
            document_id: document.id,
        })
        .await
    {
        error!("Failed to enqueue document processing job: {}", e);
    }

    // Re-fetch to get updated document with file_path
    let document = state
        .document_repo
        .find_by_id(document.id)
        .await?
        .unwrap_or(document);

    // Fetch all documents for the list update
    let documents = state.document_repo.list(None).await?;

    let template = UploadSuccessTemplate {
        document,
        documents,
    };

    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error rendering upload success: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/vnd.turbo-stream.html")], html).into_response())
}

/// Delete success template (Turbo Stream response)
#[derive(Template)]
#[template(path = "partials/delete_success.html")]
struct DeleteSuccessTemplate {
    document_id: Uuid,
}

/// Delete a document (returns Turbo Stream)
async fn delete_document(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    // Verify document exists
    let _ = state
        .document_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))?;

    // Delete the document
    state.document_repo.delete(id).await?;

    let template = DeleteSuccessTemplate { document_id: id };

    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error rendering delete success: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/vnd.turbo-stream.html")], html).into_response())
}

/// Reprocess a document (re-run OCR, thumbnails, embeddings)
async fn reprocess_document(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    debug!("Reprocess requested for document {}", id);

    // Verify document exists and has a file
    let document = state
        .document_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))?;

    debug!(
        "Document found: title={}, file_path={:?}, mime_type={:?}",
        document.title, document.file_path, document.mime_type
    );

    if document.file_path.is_none() {
        error!("Cannot reprocess document {} - no file path", id);
        return Err(ApiError::bad_request("Document has no file to process"));
    }

    // Queue a reprocessing job
    debug!("Queuing ProcessDocument job for {}", id);
    state
        .job_queue
        .enqueue(JobType::ProcessDocument { document_id: id })
        .await
        .map_err(|e| {
            error!("Failed to queue reprocessing job for {}: {}", id, e);
            ApiError::internal(format!("Failed to queue reprocessing: {}", e))
        })?;

    debug!("Successfully queued reprocessing job for document {}", id);

    // Redirect back to document with a success message
    Ok(Redirect::to(&format!("/documents/{}", id)).into_response())
}

/// Search query parameters
#[derive(Debug, serde::Deserialize)]
pub struct SearchQuery {
    q: String,
}

/// Search result with percentage score
pub struct SearchResultView {
    pub document: Document,
    pub score_percent: String,
}

/// Search results template
#[derive(Template)]
#[template(path = "partials/search_results.html")]
struct SearchResultsTemplate {
    results: Vec<SearchResultView>,
}

/// Search documents (returns Turbo Frame)
async fn search_documents(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Html<String>> {
    // Try to generate embedding for query - fall back to empty results if AI not available
    let results = match state.semantic_search(&query.q, 10).await {
        Ok(search_results) => {
            // Fetch full document details and format scores
            let mut results = Vec::new();
            for result in search_results {
                if let Ok(Some(document)) = state.document_repo.find_by_id(result.document_id).await
                {
                    results.push(SearchResultView {
                        document,
                        score_percent: format!("{:.1}", result.score * 100.0),
                    });
                }
            }
            results
        }
        Err(_) => {
            // AI not configured - fall back to simple title search
            let all_docs = state.document_repo.list(None).await.unwrap_or_default();
            let query_lower = query.q.to_lowercase();
            all_docs
                .into_iter()
                .filter(|doc| doc.title.to_lowercase().contains(&query_lower))
                .map(|document| SearchResultView {
                    document,
                    score_percent: "100.0".to_string(),
                })
                .collect()
        }
    };

    let template = SearchResultsTemplate { results };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering search results: {}", e)
    })))
}

// =============================================================================
// Document metadata form handlers (for page forms with redirects)
// =============================================================================

/// Form data for setting correspondent
#[derive(Debug, serde::Deserialize)]
pub struct SetCorrespondentForm {
    correspondent_id: Option<String>,
}

/// Set document correspondent (form handler with redirect)
async fn set_document_correspondent(
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
#[derive(Debug, serde::Deserialize)]
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
#[derive(Debug, serde::Deserialize)]
pub struct AddTagForm {
    tag_id: Option<String>,
}

/// Add a tag to a document (form handler with redirect)
async fn add_document_tag(
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
#[derive(Debug, serde::Deserialize)]
pub struct RemoveTagPath {
    id: Uuid,
    tag_id: Uuid,
}

/// Remove a tag from a document (form handler with redirect)
async fn remove_document_tag(
    State(state): State<AppState>,
    Path(path): Path<RemoveTagPath>,
) -> ApiResult<Response> {
    state.document_repo.remove_tag(path.id, path.tag_id).await?;

    Ok(Redirect::to(&format!("/documents/{}", path.id)).into_response())
}
