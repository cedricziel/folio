//! Management page routes for correspondents, document types, and tags

use askama::Template;
use axum::{
    extract::{Path, State},
    http::header,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Form, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use folio_core::models::{
    Correspondent, CreateCorrespondent, CreateDocumentType, CreateTag, DocumentType, Tag,
    UpdateCorrespondent, UpdateDocumentType, UpdateTag,
};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // Main management page
        .route("/manage", get(manage_page))
        // Correspondents
        .route("/manage/correspondents", get(correspondents_list))
        .route(
            "/manage/correspondents/create",
            post(create_correspondent_form),
        )
        .route(
            "/manage/correspondents/:id/edit",
            get(edit_correspondent_form),
        )
        .route(
            "/manage/correspondents/:id/update",
            post(update_correspondent_form),
        )
        .route(
            "/manage/correspondents/:id/delete",
            post(delete_correspondent_form),
        )
        // Document Types
        .route("/manage/document_types", get(document_types_list))
        .route(
            "/manage/document_types/create",
            post(create_document_type_form),
        )
        .route(
            "/manage/document_types/:id/edit",
            get(edit_document_type_form),
        )
        .route(
            "/manage/document_types/:id/update",
            post(update_document_type_form),
        )
        .route(
            "/manage/document_types/:id/delete",
            post(delete_document_type_form),
        )
        // Tags
        .route("/manage/tags", get(tags_list))
        .route("/manage/tags/create", post(create_tag_form))
        .route("/manage/tags/:id/edit", get(edit_tag_form))
        .route("/manage/tags/:id/update", post(update_tag_form))
        .route("/manage/tags/:id/delete", post(delete_tag_form))
}

// ============================================================================
// Main Management Page
// ============================================================================

#[derive(Template)]
#[template(path = "manage.html")]
struct ManageTemplate {
    title: String,
    correspondents: Vec<Correspondent>,
    document_types: Vec<DocumentType>,
    tags: Vec<Tag>,
}

async fn manage_page(State(state): State<AppState>) -> ApiResult<Html<String>> {
    let correspondents = state.correspondent_repo.list().await?;
    let document_types = state.document_type_repo.list().await?;
    let tags = state.tag_repo.list().await?;

    let template = ManageTemplate {
        title: "Manage - Folio".to_string(),
        correspondents,
        document_types,
        tags,
    };

    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering page: {}", e)
    })))
}

// ============================================================================
// Correspondents
// ============================================================================

#[derive(Template)]
#[template(path = "partials/correspondents_list.html")]
struct CorrespondentsListTemplate {
    correspondents: Vec<Correspondent>,
}

async fn correspondents_list(State(state): State<AppState>) -> ApiResult<Html<String>> {
    let correspondents = state.correspondent_repo.list().await?;
    let template = CorrespondentsListTemplate { correspondents };
    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering list: {}", e)
    })))
}

#[derive(Debug, Deserialize)]
pub struct CreateCorrespondentForm {
    name: String,
}

async fn create_correspondent_form(
    State(state): State<AppState>,
    Form(form): Form<CreateCorrespondentForm>,
) -> ApiResult<Response> {
    state
        .correspondent_repo
        .create(CreateCorrespondent { name: form.name })
        .await?;

    let correspondents = state.correspondent_repo.list().await?;
    let template = CorrespondentsListTemplate { correspondents };
    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/html")], html).into_response())
}

#[derive(Template)]
#[template(path = "partials/correspondent_edit.html")]
struct CorrespondentEditTemplate {
    correspondent: Correspondent,
}

async fn edit_correspondent_form(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Html<String>> {
    let correspondent = state
        .correspondent_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Correspondent not found"))?;

    let template = CorrespondentEditTemplate { correspondent };
    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering form: {}", e)
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateCorrespondentForm {
    name: String,
}

async fn update_correspondent_form(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(form): Form<UpdateCorrespondentForm>,
) -> ApiResult<Response> {
    state
        .correspondent_repo
        .update(
            id,
            UpdateCorrespondent {
                name: Some(form.name),
            },
        )
        .await?;

    let correspondents = state.correspondent_repo.list().await?;
    let template = CorrespondentsListTemplate { correspondents };
    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/html")], html).into_response())
}

async fn delete_correspondent_form(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    state.correspondent_repo.delete(id).await?;

    let correspondents = state.correspondent_repo.list().await?;
    let template = CorrespondentsListTemplate { correspondents };
    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/html")], html).into_response())
}

// ============================================================================
// Document Types
// ============================================================================

#[derive(Template)]
#[template(path = "partials/document_types_list.html")]
struct DocumentTypesListTemplate {
    document_types: Vec<DocumentType>,
}

async fn document_types_list(State(state): State<AppState>) -> ApiResult<Html<String>> {
    let document_types = state.document_type_repo.list().await?;
    let template = DocumentTypesListTemplate { document_types };
    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering list: {}", e)
    })))
}

#[derive(Debug, Deserialize)]
pub struct CreateDocumentTypeForm {
    name: String,
}

async fn create_document_type_form(
    State(state): State<AppState>,
    Form(form): Form<CreateDocumentTypeForm>,
) -> ApiResult<Response> {
    state
        .document_type_repo
        .create(CreateDocumentType { name: form.name })
        .await?;

    let document_types = state.document_type_repo.list().await?;
    let template = DocumentTypesListTemplate { document_types };
    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/html")], html).into_response())
}

#[derive(Template)]
#[template(path = "partials/document_type_edit.html")]
struct DocumentTypeEditTemplate {
    document_type: DocumentType,
}

async fn edit_document_type_form(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Html<String>> {
    let document_type = state
        .document_type_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document type not found"))?;

    let template = DocumentTypeEditTemplate { document_type };
    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering form: {}", e)
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateDocumentTypeForm {
    name: String,
}

async fn update_document_type_form(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(form): Form<UpdateDocumentTypeForm>,
) -> ApiResult<Response> {
    state
        .document_type_repo
        .update(
            id,
            UpdateDocumentType {
                name: Some(form.name),
            },
        )
        .await?;

    let document_types = state.document_type_repo.list().await?;
    let template = DocumentTypesListTemplate { document_types };
    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/html")], html).into_response())
}

async fn delete_document_type_form(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    state.document_type_repo.delete(id).await?;

    let document_types = state.document_type_repo.list().await?;
    let template = DocumentTypesListTemplate { document_types };
    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/html")], html).into_response())
}

// ============================================================================
// Tags
// ============================================================================

#[derive(Template)]
#[template(path = "partials/tags_list.html")]
struct TagsListTemplate {
    tags: Vec<Tag>,
}

async fn tags_list(State(state): State<AppState>) -> ApiResult<Html<String>> {
    let tags = state.tag_repo.list().await?;
    let template = TagsListTemplate { tags };
    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering list: {}", e)
    })))
}

#[derive(Debug, Deserialize)]
pub struct CreateTagForm {
    name: String,
    color: Option<String>,
}

async fn create_tag_form(
    State(state): State<AppState>,
    Form(form): Form<CreateTagForm>,
) -> ApiResult<Response> {
    let color = form.color.filter(|c| !c.is_empty());
    state
        .tag_repo
        .create(CreateTag {
            name: form.name,
            color,
        })
        .await?;

    let tags = state.tag_repo.list().await?;
    let template = TagsListTemplate { tags };
    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/html")], html).into_response())
}

#[derive(Template)]
#[template(path = "partials/tag_edit.html")]
struct TagEditTemplate {
    tag: Tag,
}

async fn edit_tag_form(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Html<String>> {
    let tag = state
        .tag_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Tag not found"))?;

    let template = TagEditTemplate { tag };
    Ok(Html(template.render().unwrap_or_else(|e| {
        format!("Error rendering form: {}", e)
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateTagForm {
    name: String,
    color: Option<String>,
}

async fn update_tag_form(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(form): Form<UpdateTagForm>,
) -> ApiResult<Response> {
    let color = if form.color.as_ref().map(|c| c.is_empty()).unwrap_or(true) {
        Some(None)
    } else {
        Some(form.color)
    };

    state
        .tag_repo
        .update(
            id,
            UpdateTag {
                name: Some(form.name),
                color,
            },
        )
        .await?;

    let tags = state.tag_repo.list().await?;
    let template = TagsListTemplate { tags };
    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/html")], html).into_response())
}

async fn delete_tag_form(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Response> {
    state.tag_repo.delete(id).await?;

    let tags = state.tag_repo.list().await?;
    let template = TagsListTemplate { tags };
    let html = template
        .render()
        .unwrap_or_else(|e| format!("Error: {}", e));

    Ok(([(header::CONTENT_TYPE, "text/html")], html).into_response())
}
