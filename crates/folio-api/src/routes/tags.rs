//! Tag API routes

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use folio_core::models::{CreateTag, Tag, UpdateTag};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_tags).post(create_tag))
        .route("/{id}", get(get_tag).put(update_tag).delete(delete_tag))
}

/// List all tags
async fn list_tags(State(state): State<AppState>) -> ApiResult<Json<Vec<Tag>>> {
    let tags = state.tag_repo.list().await?;
    Ok(Json(tags))
}

/// Get a single tag by ID
async fn get_tag(State(state): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Json<Tag>> {
    let tag = state
        .tag_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Tag not found"))?;

    Ok(Json(tag))
}

/// Request body for creating a tag
#[derive(Debug, Deserialize)]
pub struct CreateTagRequest {
    name: String,
    color: Option<String>,
}

/// Create a new tag
async fn create_tag(
    State(state): State<AppState>,
    Json(req): Json<CreateTagRequest>,
) -> ApiResult<Json<Tag>> {
    let tag = state
        .tag_repo
        .create(CreateTag {
            name: req.name,
            color: req.color,
        })
        .await?;

    Ok(Json(tag))
}

/// Request body for updating a tag
#[derive(Debug, Deserialize)]
pub struct UpdateTagRequest {
    name: Option<String>,
    color: Option<Option<String>>,
}

/// Update a tag
async fn update_tag(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTagRequest>,
) -> ApiResult<Json<Tag>> {
    let tag = state
        .tag_repo
        .update(
            id,
            UpdateTag {
                name: req.name,
                color: req.color,
            },
        )
        .await?;

    Ok(Json(tag))
}

/// Delete a tag
async fn delete_tag(State(state): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Json<()>> {
    state.tag_repo.delete(id).await?;
    Ok(Json(()))
}
