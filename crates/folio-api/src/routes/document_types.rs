//! Document Type API routes

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use folio_core::models::{CreateDocumentType, DocumentType, UpdateDocumentType};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_document_types).post(create_document_type))
        .route(
            "/{id}",
            get(get_document_type)
                .put(update_document_type)
                .delete(delete_document_type),
        )
}

/// List all document types
async fn list_document_types(State(state): State<AppState>) -> ApiResult<Json<Vec<DocumentType>>> {
    let document_types = state.document_type_repo.list().await?;
    Ok(Json(document_types))
}

/// Get a single document type by ID
async fn get_document_type(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DocumentType>> {
    let document_type = state
        .document_type_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document type not found"))?;

    Ok(Json(document_type))
}

/// Request body for creating a document type
#[derive(Debug, Deserialize)]
pub struct CreateDocumentTypeRequest {
    name: String,
}

/// Create a new document type
async fn create_document_type(
    State(state): State<AppState>,
    Json(req): Json<CreateDocumentTypeRequest>,
) -> ApiResult<Json<DocumentType>> {
    let document_type = state
        .document_type_repo
        .create(CreateDocumentType { name: req.name })
        .await?;

    Ok(Json(document_type))
}

/// Request body for updating a document type
#[derive(Debug, Deserialize)]
pub struct UpdateDocumentTypeRequest {
    name: Option<String>,
}

/// Update a document type
async fn update_document_type(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateDocumentTypeRequest>,
) -> ApiResult<Json<DocumentType>> {
    let document_type = state
        .document_type_repo
        .update(id, UpdateDocumentType { name: req.name })
        .await?;

    Ok(Json(document_type))
}

/// Delete a document type
async fn delete_document_type(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<()>> {
    state.document_type_repo.delete(id).await?;
    Ok(Json(()))
}
