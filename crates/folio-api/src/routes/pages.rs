//! Page routes for Turbo-based web UI

use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::header,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use tracing::{debug, error};
use uuid::Uuid;

use folio_core::models::{CreateDocument, Document};

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
        .route("/documents/:id/delete", post(delete_document))
}

/// Index page template
#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    title: String,
    documents: Vec<Document>,
}

/// Main index page
async fn index_page(State(state): State<AppState>) -> ApiResult<Html<String>> {
    let documents = state.document_repo.list(None).await?;

    let template = IndexTemplate {
        title: "Folio - Document Management".to_string(),
        documents,
    };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering page: {}", e)
    })))
}

/// Documents list template (Turbo Frame partial)
#[derive(Template)]
#[template(path = "partials/documents_list.html")]
struct DocumentsListTemplate {
    documents: Vec<Document>,
}

/// Documents list (for Turbo Frame)
async fn documents_list(State(state): State<AppState>) -> ApiResult<Html<String>> {
    let documents = state.document_repo.list(None).await?;

    let template = DocumentsListTemplate { documents };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering list: {}", e)
    })))
}

/// Document detail template (Turbo Frame partial)
#[derive(Template)]
#[template(path = "document_detail.html")]
struct DocumentDetailTemplate {
    document: Document,
}

/// Document detail (for Turbo Frame)
async fn document_detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Html<String>> {
    let document = state
        .document_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| crate::error::ApiError::not_found("Document not found"))?;

    let template = DocumentDetailTemplate { document };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering detail: {}", e)
    })))
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
    let results = match state.embedding_service.generate_embedding(&query.q).await {
        Ok(query_embedding) => {
            // Search for similar documents
            let search_results = state
                .search_service
                .search(&query_embedding, 10)
                .await
                .unwrap_or_default();

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
