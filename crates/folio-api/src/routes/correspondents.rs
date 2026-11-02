//! Correspondent API routes

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use folio_core::models::{Correspondent, CreateCorrespondent, UpdateCorrespondent};

use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath};
use crate::openapi::{BadRequest, Conflict, InternalError, NotFound};
use crate::state::AppState;
use crate::vault::DefaultVault;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_correspondents).post(create_correspondent))
        .route(
            "/{id}",
            get(get_correspondent)
                .put(update_correspondent)
                .delete(delete_correspondent),
        )
}

/// List all correspondents
#[utoipa::path(
    get,
    path = "/api/correspondents",
    operation_id = "listCorrespondents",
    tag = "correspondents",
    responses(
        (status = 200, description = "All correspondents", body = Vec<Correspondent>),
        InternalError,
    )
)]
pub(crate) async fn list_correspondents(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<Correspondent>>> {
    let correspondents = state.correspondent_repo.list(vault).await?;
    Ok(Json(correspondents))
}

/// Get a single correspondent by ID
#[utoipa::path(
    get,
    path = "/api/correspondents/{id}",
    operation_id = "getCorrespondent",
    tag = "correspondents",
    params(("id" = Uuid, Path, description = "Correspondent ID")),
    responses(
        (status = 200, description = "The correspondent", body = Correspondent),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn get_correspondent(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<Correspondent>> {
    let correspondent = state
        .correspondent_repo
        .find_by_id(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Correspondent not found"))?;

    Ok(Json(correspondent))
}

/// Request body for creating a correspondent
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCorrespondentRequest {
    name: String,
}

/// Create a new correspondent
#[utoipa::path(
    post,
    path = "/api/correspondents",
    operation_id = "createCorrespondent",
    tag = "correspondents",
    request_body = CreateCorrespondentRequest,
    responses(
        (status = 200, description = "The created correspondent", body = Correspondent),
        BadRequest,
        Conflict,
        InternalError,
    )
)]
pub(crate) async fn create_correspondent(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiJson(req): ApiJson<CreateCorrespondentRequest>,
) -> ApiResult<Json<Correspondent>> {
    let correspondent = state
        .correspondent_repo
        .create(vault, CreateCorrespondent { name: req.name })
        .await?;

    Ok(Json(correspondent))
}

/// Request body for updating a correspondent
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateCorrespondentRequest {
    name: Option<String>,
}

/// Update a correspondent
#[utoipa::path(
    put,
    path = "/api/correspondents/{id}",
    operation_id = "updateCorrespondent",
    tag = "correspondents",
    params(("id" = Uuid, Path, description = "Correspondent ID")),
    request_body = UpdateCorrespondentRequest,
    responses(
        (status = 200, description = "The updated correspondent", body = Correspondent),
        BadRequest,
        NotFound,
        Conflict,
        InternalError,
    )
)]
pub(crate) async fn update_correspondent(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<UpdateCorrespondentRequest>,
) -> ApiResult<Json<Correspondent>> {
    let correspondent = state
        .correspondent_repo
        .update(vault, id, UpdateCorrespondent { name: req.name })
        .await?;

    Ok(Json(correspondent))
}

/// Delete a correspondent
#[utoipa::path(
    delete,
    path = "/api/correspondents/{id}",
    operation_id = "deleteCorrespondent",
    tag = "correspondents",
    params(("id" = Uuid, Path, description = "Correspondent ID")),
    responses(
        (status = 204, description = "The correspondent was deleted"),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn delete_correspondent(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    state.correspondent_repo.delete(vault, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
