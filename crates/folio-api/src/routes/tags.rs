//! Tag API routes

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use folio_core::models::{CreateTag, Tag, UpdateTag};

use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath};
use crate::openapi::{BadRequest, Conflict, InternalError, NotFound};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_tags).post(create_tag))
        .route("/{id}", get(get_tag).put(update_tag).delete(delete_tag))
}

/// List all tags
#[utoipa::path(
    get,
    path = "/api/tags",
    operation_id = "listTags",
    tag = "tags",
    responses(
        (status = 200, description = "All tags", body = Vec<Tag>),
        InternalError,
    )
)]
pub(crate) async fn list_tags(State(state): State<AppState>) -> ApiResult<Json<Vec<Tag>>> {
    let tags = state.tag_repo.list().await?;
    Ok(Json(tags))
}

/// Get a single tag by ID
#[utoipa::path(
    get,
    path = "/api/tags/{id}",
    operation_id = "getTag",
    tag = "tags",
    params(("id" = Uuid, Path, description = "Tag ID")),
    responses(
        (status = 200, description = "The tag", body = Tag),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn get_tag(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<Tag>> {
    let tag = state
        .tag_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("Tag not found"))?;

    Ok(Json(tag))
}

/// Request body for creating a tag
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTagRequest {
    name: String,
    color: Option<String>,
}

/// Create a new tag
#[utoipa::path(
    post,
    path = "/api/tags",
    operation_id = "createTag",
    tag = "tags",
    request_body = CreateTagRequest,
    responses(
        (status = 200, description = "The created tag", body = Tag),
        BadRequest,
        Conflict,
        InternalError,
    )
)]
pub(crate) async fn create_tag(
    State(state): State<AppState>,
    ApiJson(req): ApiJson<CreateTagRequest>,
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
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTagRequest {
    name: Option<String>,
    /// Omit to keep the current color; send `null` to clear it
    #[serde(default, deserialize_with = "deserialize_some")]
    #[schema(value_type = Option<String>)]
    color: Option<Option<String>>,
}

/// Distinguishes an explicit `null` (`Some(None)`) from a missing field (`None`).
fn deserialize_some<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// Update a tag
#[utoipa::path(
    put,
    path = "/api/tags/{id}",
    operation_id = "updateTag",
    tag = "tags",
    params(("id" = Uuid, Path, description = "Tag ID")),
    request_body = UpdateTagRequest,
    responses(
        (status = 200, description = "The updated tag", body = Tag),
        BadRequest,
        NotFound,
        Conflict,
        InternalError,
    )
)]
pub(crate) async fn update_tag(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(req): ApiJson<UpdateTagRequest>,
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
#[utoipa::path(
    delete,
    path = "/api/tags/{id}",
    operation_id = "deleteTag",
    tag = "tags",
    params(("id" = Uuid, Path, description = "Tag ID")),
    responses(
        (status = 204, description = "The tag was deleted"),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn delete_tag(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    state.tag_repo.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
