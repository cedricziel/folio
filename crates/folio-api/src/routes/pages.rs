//! Page routes for Turbo-based web UI

use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use tracing::{debug, error};
use uuid::Uuid;

use folio_core::models::{
    Correspondent, Document, DocumentSource, DocumentStatus, DocumentType, SourceKind,
    UpdateDocument,
};
use folio_core::ports::{keys, DocumentStore, StoredObject};
use folio_core::services::NewUpload;
use folio_infra::JobType;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

use super::file_response::{harden, harden_inline};
use super::view::{self, Counts, DocumentRow, Nav, Sidebar, TagView};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(index_page))
        .route("/documents", get(documents_list))
        // Static routes MUST come before parameterized routes
        .route("/documents/upload", post(upload_document))
        .route("/documents/search", get(search_documents))
        // Parameterized routes come last
        .route("/documents/{id}", get(document_detail))
        .route("/documents/{id}/thumbnail", get(document_thumbnail))
        .route(
            "/documents/{id}/page/{page}/thumbnail",
            get(document_page_thumbnail),
        )
        .route("/documents/{id}/delete", post(delete_document))
        .route(
            "/documents/{id}/correspondent",
            post(set_document_correspondent),
        )
        .route("/documents/{id}/document_type", post(set_document_type))
        .route("/documents/{id}/tags", post(add_document_tag))
        .route("/documents/{id}/tags/{tag_id}", post(remove_document_tag))
        .route("/documents/{id}/reprocess", post(reprocess_document))
}

/// Index page template
#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    sidebar: Sidebar,
    heading: &'static str,
    lede: &'static str,
    view_all: bool,
    active_filter: Option<String>,
    rows: Vec<DocumentRow>,
    total: usize,
    counts: Counts,
}

/// Query parameters for the index page
#[derive(Debug, serde::Deserialize, Default)]
pub struct FilterQuery {
    view: Option<String>,
    correspondent_id: Option<String>,
    document_type_id: Option<String>,
    tag_id: Option<String>,
}

fn parse_id(value: &Option<String>) -> Option<Uuid> {
    value
        .as_ref()
        .filter(|s| !s.is_empty())
        .and_then(|s| Uuid::parse_str(s).ok())
}

/// Main index page: the inbox, or every document when a view or filter asks for it
async fn index_page(
    State(state): State<AppState>,
    Query(query): Query<FilterQuery>,
) -> ApiResult<Html<String>> {
    use folio_core::models::DocumentFilter;

    let correspondent_id = parse_id(&query.correspondent_id);
    let document_type_id = parse_id(&query.document_type_id);
    let tag_id = parse_id(&query.tag_id);
    let filtered = correspondent_id.is_some() || document_type_id.is_some() || tag_id.is_some();
    let view_all = filtered || query.view.as_deref() == Some("all");

    let all = state.document_repo.list(None).await?;
    let sidebar =
        Sidebar::build(&state, if view_all { Nav::All } else { Nav::Inbox }, &all).await?;
    let counts = Counts::from_documents(&all);

    let active_filter = correspondent_id
        .and_then(|id| sidebar.correspondents.iter().find(|c| c.id == id))
        .map(|c| c.name.clone())
        .or_else(|| {
            document_type_id
                .and_then(|id| sidebar.document_types.iter().find(|t| t.id == id))
                .map(|t| t.name.clone())
        })
        .or_else(|| {
            tag_id
                .and_then(|id| sidebar.tags.iter().find(|t| t.id == id))
                .map(|t| t.name.clone())
        });

    let documents = if filtered {
        state
            .document_repo
            .list(Some(DocumentFilter {
                correspondent_id,
                document_type_id,
                tag_ids: tag_id.map(|id| vec![id]),
                ..Default::default()
            }))
            .await?
    } else if view_all {
        all
    } else {
        all.into_iter()
            .filter(|d| d.status() != DocumentStatus::Filed)
            .collect()
    };

    let (heading, lede) = if view_all {
        ("All documents", "Everything Folio holds, newest first.")
    } else {
        (
            "Inbox",
            "New files land here until you have had a look at them.",
        )
    };

    let template = IndexTemplate {
        sidebar,
        heading,
        lede,
        view_all,
        active_filter,
        rows: view::rows(&state, documents).await?,
        total: counts.total,
        counts,
    };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering page: {}", e)
    })))
}

/// Documents list template (Turbo Frame partial)
#[derive(Template)]
#[template(path = "partials/documents_frame.html")]
struct DocumentsFrameTemplate {
    rows: Vec<DocumentRow>,
    total: usize,
    view_all: bool,
}

/// Documents list (for Turbo Frame)
async fn documents_list(State(state): State<AppState>) -> ApiResult<Html<String>> {
    let documents = state.document_repo.list(None).await?;
    let total = documents.len();

    let template = DocumentsFrameTemplate {
        rows: view::rows(&state, documents).await?,
        total,
        view_all: true,
    };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering list: {}", e)
    })))
}

/// Document view template (full page)
#[derive(Template)]
#[template(path = "document_view.html")]
struct DocumentViewTemplate {
    sidebar: Sidebar,
    document: Document,
    facts: String,
    reading: String,
    failed: bool,
    pages: Vec<u32>,
    correspondent: Option<Correspondent>,
    document_type: Option<DocumentType>,
    tags: Vec<TagView>,
    all_correspondents: Vec<Correspondent>,
    all_document_types: Vec<DocumentType>,
    all_tags: Vec<TagView>,
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

    let tags: Vec<TagView> = state
        .document_repo
        .get_tags(document.id)
        .await?
        .iter()
        .map(TagView::from)
        .collect();

    // Fetch all options for editing
    let all_correspondents = state.correspondent_repo.list().await?;
    let all_document_types = state.document_type_repo.list().await?;
    let all_tags: Vec<TagView> = state
        .tag_repo
        .list()
        .await?
        .iter()
        .map(TagView::from)
        .collect();

    let show_page_nav = document.page_count.map(|c| c > 1).unwrap_or(false);
    let pages: Vec<u32> = (1..=document.page_count.unwrap_or(1).max(1)).collect();

    let mut facts = Vec::new();
    // A file that could not be read has no trustworthy page count
    let failed = document.status() == DocumentStatus::Failed;
    if let Some(count) = document.page_count.filter(|_| !failed) {
        facts.push(format!(
            "{count} {}",
            if count == 1 { "page" } else { "pages" }
        ));
    }
    if let Some(kind) = document.mime_type.as_deref().and_then(kind_label) {
        facts.push(kind.to_string());
    }
    facts.push(format!("added {}", document.created_at.format("%Y-%m-%d")));

    let template = DocumentViewTemplate {
        sidebar: Sidebar::load(&state, Nav::None).await?,
        facts: facts.join(" · "),
        reading: view::reading_label(document.page_count),
        failed,
        pages,
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

/// What people call the file: "PDF" or "Photo", not the MIME type
fn kind_label(mime: &str) -> Option<&'static str> {
    match mime {
        "application/pdf" => Some("PDF"),
        m if m.starts_with("image/") => Some("Photo"),
        _ => None,
    }
}

/// The first of `candidates` that exists; later ones are keys from before the
/// object-key layout, kept so thumbnails survive until the layout migration ran.
async fn first_object(store: &dyn DocumentStore, candidates: &[String]) -> Option<StoredObject> {
    for key in candidates {
        match store.get(key).await {
            Ok(object) => return Some(object),
            Err(folio_core::Error::NotFound { .. }) => {}
            Err(e) => error!("Failed to read {key}: {e}"),
        }
    }
    None
}

/// Stream a stored object as a response body
fn stream_object(object: StoredObject, content_type: &str) -> Response {
    let mut response = Response::builder().header(header::CONTENT_TYPE, content_type);
    if let Some(size) = object.size {
        response = response.header(header::CONTENT_LENGTH, size);
    }
    let mut response = response
        .body(axum::body::Body::from_stream(object.stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    harden_inline(&mut response, content_type);
    response
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
    if let Some(object) = first_object(
        state.store.as_ref(),
        &[keys::thumbnail(id), keys::legacy::thumbnail(id)],
    )
    .await
    {
        return Ok(stream_object(object, "image/jpeg"));
    }

    // Fallback: try to serve the original image if it's an image
    let is_image = document
        .mime_type
        .as_ref()
        .map(|m| m.starts_with("image/"))
        .unwrap_or(false);

    if is_image {
        if let Some(ref file_path) = document.file_path {
            if let Ok(object) = state.store.get(file_path).await {
                let mime = document
                    .mime_type
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                return Ok(stream_object(object, &mime));
            }
        }
    }

    // Non-images and missing files get a quiet sheet, not a loud file-type icon
    Ok(placeholder_svg(placeholder_sheet(100, 100)))
}

/// A transparent sheet with a small document outline, in `ink-faint`, readable on both grounds
fn placeholder_sheet(width: u32, height: u32) -> String {
    let (x, y) = (width / 2 - 9, height / 2 - 12);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">
            <g transform="translate({x} {y})" fill="none" stroke="#7a7f7a" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round">
                <path d="M2 1h10l6 6v16H2z"/>
                <path d="M12 1v6h6"/>
            </g>
        </svg>"##
    )
}

/// Static SVG generated by the server (inline is safe, the sandbox CSP still applies)
fn placeholder_svg(svg: impl Into<String>) -> Response {
    let mut response = ([(header::CONTENT_TYPE, "image/svg+xml")], svg.into()).into_response();
    harden(&mut response);
    response
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
    if let Some(object) = first_object(
        state.store.as_ref(),
        &[keys::page(id, page), keys::legacy::page(id, page)],
    )
    .await
    {
        return Ok(stream_object(object, "image/jpeg"));
    }

    // Fallback: a quiet placeholder for the page
    Ok(placeholder_svg(placeholder_sheet(100, 140)))
}

/// Turbo Streams that bring a list and the counters up to date
#[derive(Template)]
#[template(path = "partials/document_streams.html")]
pub(super) struct DocumentStreams {
    row: Option<DocumentRow>,
    prepend: bool,
    removed: Option<Uuid>,
    counts: Counts,
}

impl DocumentStreams {
    pub(super) async fn added(state: &AppState, document: Document) -> ApiResult<Self> {
        Ok(Self {
            row: Some(view::row(state, document).await?),
            prepend: true,
            removed: None,
            counts: Counts::load(state).await?,
        })
    }

    pub(super) async fn changed(state: &AppState, document: Document) -> ApiResult<Self> {
        Ok(Self {
            row: Some(view::row(state, document).await?),
            prepend: false,
            removed: None,
            counts: Counts::load(state).await?,
        })
    }

    pub(super) async fn removed(state: &AppState, id: Uuid) -> ApiResult<Self> {
        Ok(Self {
            row: None,
            prepend: false,
            removed: Some(id),
            counts: Counts::load(state).await?,
        })
    }

    pub(super) fn into_html(self) -> String {
        self.render()
            .unwrap_or_else(|e| format!("Error rendering streams: {}", e))
    }
}

/// Read the uploaded file out of a multipart body, store it and queue processing
pub(crate) async fn store_upload(
    state: &AppState,
    mut multipart: Multipart,
    source: SourceKind,
) -> ApiResult<Document> {
    debug!("Starting document upload");

    let mut title: Option<String> = None;
    let mut file_data: Option<Vec<u8>> = None;
    let mut filename: Option<String> = None;
    let mut mime_type: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| multipart_error("Invalid multipart data", e))?
    {
        let name = field.name().unwrap_or("").to_string();
        debug!("Processing field: {}", name);

        match name.as_str() {
            "title" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| multipart_error("Invalid title", e))?;
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
                        .map_err(|e| multipart_error("Failed to read file", e))?
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

    // Storing and queueing (OCR, thumbnails, embedding) is shared with ingestion
    let document = state
        .importer
        .import(NewUpload {
            title: Some(title),
            filename: filename.unwrap_or_else(|| "upload".to_string()),
            mime_type: Some(mime_type),
            data: file_data,
            source: DocumentSource::of(source),
            tag_ids: Vec::new(),
        })
        .await?;

    folio_infra::telemetry::metrics().documents_changed(1);

    Ok(document)
}

/// Keeps the status the multipart parser chose, such as 413 for an oversized body
fn multipart_error(context: &str, e: axum::extract::multipart::MultipartError) -> ApiError {
    error!("Multipart error: {}", e);
    ApiError::new(e.status(), format!("{context}: {}", e.body_text()))
}

/// Upload a document (returns Turbo Stream)
async fn upload_document(
    State(state): State<AppState>,
    multipart: Multipart,
) -> ApiResult<Response> {
    let document = store_upload(&state, multipart, SourceKind::Upload).await?;
    let html = DocumentStreams::added(&state, document).await?.into_html();

    Ok(([(header::CONTENT_TYPE, "text/vnd.turbo-stream.html")], html).into_response())
}

/// Delete a document, then go back to the inbox
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

    // Delete the stored files first so a failure leaves the row to retry
    super::documents::purge_document_files(&state, id).await?;
    state.document_repo.delete(id).await?;
    folio_infra::telemetry::metrics().documents_changed(-1);

    Ok(Redirect::to("/").into_response())
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

/// Search results template
#[derive(Template)]
#[template(path = "partials/search_results.html")]
struct SearchResultsTemplate {
    rows: Vec<DocumentRow>,
}

/// Search documents (returns Turbo Frame)
async fn search_documents(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Html<String>> {
    let mut documents = Vec::new();
    for result in state.search_documents(&query.q, 10).await? {
        if let Ok(Some(document)) = state.document_repo.find_by_id(result.document_id).await {
            documents.push(document);
        }
    }

    let template = SearchResultsTemplate {
        rows: view::rows(&state, documents).await?,
    };

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
