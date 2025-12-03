//! Correspondent API routes

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use folio_core::models::{Correspondent, CreateCorrespondent, UpdateCorrespondent};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

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
async fn list_correspondents(State(state): State<AppState>) -> ApiResult<Json<Vec<Correspondent>>> {
    let correspondents = state.correspondent_repo.list().await?;
    Ok(Json(correspondents))
}

/// Get a single correspondent by ID
async fn get_correspondent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Correspondent>> {
    let correspondent = state
        .correspondent_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Correspondent not found"))?;

    Ok(Json(correspondent))
}

/// Request body for creating a correspondent
#[derive(Debug, Deserialize)]
pub struct CreateCorrespondentRequest {
    name: String,
}

/// Create a new correspondent
async fn create_correspondent(
    State(state): State<AppState>,
    Json(req): Json<CreateCorrespondentRequest>,
) -> ApiResult<Json<Correspondent>> {
    let correspondent = state
        .correspondent_repo
        .create(CreateCorrespondent { name: req.name })
        .await?;

    Ok(Json(correspondent))
}

/// Request body for updating a correspondent
#[derive(Debug, Deserialize)]
pub struct UpdateCorrespondentRequest {
    name: Option<String>,
}

/// Update a correspondent
async fn update_correspondent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateCorrespondentRequest>,
) -> ApiResult<Json<Correspondent>> {
    let correspondent = state
        .correspondent_repo
        .update(id, UpdateCorrespondent { name: req.name })
        .await?;

    Ok(Json(correspondent))
}

/// Delete a correspondent
async fn delete_correspondent(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<()>> {
    state.correspondent_repo.delete(id).await?;
    Ok(Json(()))
}
