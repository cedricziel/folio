//! Document Type API routes

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use folio_core::models::{CreateDocumentType, DocumentType, UpdateDocumentType};

use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath};
use crate::openapi::{BadRequest, Conflict, InternalError, NotFound};
use crate::state::AppState;
use crate::vault::DefaultVault;

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
#[utoipa::path(
    get,
    path = "/api/document_types",
    operation_id = "listDocumentTypes",
    tag = "document_types",
    responses(
        (status = 200, description = "All document types", body = Vec<DocumentType>),
        InternalError,
    )
)]
pub(crate) async fn list_document_types(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<DocumentType>>> {
    let document_types = state.document_type_repo.list(vault).await?;
    Ok(Json(document_types))
}

/// Get a single document type by ID
#[utoipa::path(
    get,
    path = "/api/document_types/{id}",
    operation_id = "getDocumentType",
    tag = "document_types",
    params(("id" = Uuid, Path, description = "Document type ID")),
    responses(
        (status = 200, description = "The document type", body = DocumentType),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn get_document_type(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<DocumentType>> {
    let document_type = state
        .document_type_repo
        .find_by_id(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document type not found"))?;

    Ok(Json(document_type))
}

/// Request body for creating a document type
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateDocumentTypeRequest {
    name: String,
}

/// Create a new document type
#[utoipa::path(
    post,
    path = "/api/document_types",
    operation_id = "createDocumentType",
    tag = "document_types",
    request_body = CreateDocumentTypeRequest,
    responses(
        (status = 200, description = "The created document type", body = DocumentType),
        BadRequest,
        Conflict,
        InternalError,
    )
)]
pub(crate) async fn create_document_type(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiJson(req): ApiJson<CreateDocumentTypeRequest>,
) -> ApiResult<Json<DocumentType>> {
    let document_type = state
        .document_type_repo
        .create(vault, CreateDocumentType { name: req.name })
        .await?;

    Ok(Json(document_type))
}

/// Request body for updating a document type
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateDocumentTypeRequest {
    name: Option<String>,
}

/// Update a document type
#[utoipa::path(
    put,
    path = "/api/document_types/{id}",
    operation_id = "updateDocumentType",
    tag = "document_types",
    params(("id" = Uuid, Path, description = "Document type ID")),
    request_body = UpdateDocumentTypeRequest,
    responses(
        (status = 200, description = "The updated document type", body = DocumentType),
        BadRequest,
        NotFound,
        Conflict,
        InternalError,
    )
)]
pub(crate) async fn update_document_type(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<UpdateDocumentTypeRequest>,
) -> ApiResult<Json<DocumentType>> {
    let document_type = state
        .document_type_repo
        .update(vault, id, UpdateDocumentType { name: req.name })
        .await?;

    Ok(Json(document_type))
}

/// Delete a document type
#[utoipa::path(
    delete,
    path = "/api/document_types/{id}",
    operation_id = "deleteDocumentType",
    tag = "document_types",
    params(("id" = Uuid, Path, description = "Document type ID")),
    responses(
        (status = 200, description = "The document type was deleted"),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn delete_document_type(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    state.document_type_repo.delete(vault, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
