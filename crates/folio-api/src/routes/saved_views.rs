use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use folio_core::models::{
    SavedView, SavedViewInput, SavedViewQuery, VaultId, VaultRole, ViewPresentation, ViewVisibility,
};
use folio_core::ports::SavedViewAccess;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    extract::{ApiJson, ApiPath, ApiQuery},
    state::AppState,
    vault::{DefaultVault, Uploader},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/vaults/{vault_id}/saved-views", get(list).post(create))
        .route(
            "/vaults/{vault_id}/saved-views/{id}",
            get(find).patch(update).delete(delete),
        )
}

pub(super) async fn access(
    state: &AppState,
    vault: VaultId,
    uploader: Uploader,
) -> ApiResult<SavedViewAccess> {
    let user = uploader
        .0
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Sign in to save views"))?;
    let role = state
        .access
        .effective_role(&user, vault)
        .await?
        .ok_or_else(|| ApiError::not_found("Vault not found"))?;
    Ok(SavedViewAccess { user, role })
}

pub(super) async fn validate(
    state: &AppState,
    vault: VaultId,
    input: &SavedViewInput,
    access: &SavedViewAccess,
) -> ApiResult<()> {
    input.validate(vault)?;
    if input.visibility == ViewVisibility::Shared && access.role < VaultRole::Editor {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Editor access is required to share a view",
        ));
    }
    validate_references(state, vault, &input.query).await
}

pub(super) async fn validate_references(
    state: &AppState,
    vault: VaultId,
    query: &SavedViewQuery,
) -> ApiResult<()> {
    query.page(vault, 50, None)?;
    let filters = &query.filters;
    if let Some(id) = filters.correspondent_id {
        if state
            .correspondent_repo
            .find_by_id(vault, id)
            .await?
            .is_none()
        {
            return Err(ApiError::bad_request(
                "A saved-view correspondent is unavailable",
            ));
        }
    }
    if let Some(id) = filters.document_type_id {
        if state
            .document_type_repo
            .find_by_id(vault, id)
            .await?
            .is_none()
        {
            return Err(ApiError::bad_request(
                "A saved-view document type is unavailable",
            ));
        }
    }
    for id in filters.tag_ids.iter().flatten() {
        if state.tag_repo.find_by_id(vault, *id).await?.is_none() {
            return Err(ApiError::bad_request("A saved-view tag is unavailable"));
        }
    }
    Ok(())
}

pub(super) fn may_edit(view: &SavedView, access: &SavedViewAccess) -> ApiResult<()> {
    if !view.editable_by(&access.user, access.role) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "You cannot edit this saved view",
        ));
    }
    Ok(())
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct ListViewsQuery {
    after: Option<Uuid>,
    #[serde(default = "page_size")]
    limit: u32,
}
fn page_size() -> u32 {
    50
}

#[utoipa::path(get, path = "/api/vaults/{vault_id}/saved-views", operation_id = "listSavedViews", tag = "saved-views",
    params(("vault_id" = Uuid, Path), ListViewsQuery),
    responses((status = 200, description = "Visible views ordered by ID; pass the last ID as after for the next page", body = Vec<SavedView>), (status = 401, description = "Sign-in required"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::InternalError))]
pub(crate) async fn list(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<ListViewsQuery>,
) -> ApiResult<Json<Vec<SavedView>>> {
    let access = access(&state, vault, uploader).await?;
    Ok(Json(
        state
            .saved_views
            .list(vault, &access, query.after, query.limit)
            .await?,
    ))
}

#[utoipa::path(post, path = "/api/vaults/{vault_id}/saved-views", operation_id = "createSavedView", tag = "saved-views",
    params(("vault_id" = Uuid, Path)), request_body = SavedViewInput,
    responses((status = 201, description = "Created saved view", body = SavedView), (status = 401, description = "Sign-in required"), (status = 403, description = "Sharing requires Editor access"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn create(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiJson(input): ApiJson<SavedViewInput>,
) -> ApiResult<(StatusCode, Json<SavedView>)> {
    let access = access(&state, vault, uploader).await?;
    validate(&state, vault, &input, &access).await?;
    Ok((
        StatusCode::CREATED,
        Json(state.saved_views.create(vault, &access, input).await?),
    ))
}

pub(super) async fn visible(
    state: &AppState,
    vault: VaultId,
    access: &SavedViewAccess,
    id: Uuid,
) -> ApiResult<SavedView> {
    state
        .saved_views
        .find(vault, access, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Saved view not found"))
}

#[utoipa::path(get, path = "/api/vaults/{vault_id}/saved-views/{id}", operation_id = "getSavedView", tag = "saved-views",
    params(("vault_id" = Uuid, Path), ("id" = Uuid, Path)),
    responses((status = 200, description = "Saved view with validated references", body = SavedView), (status = 401, description = "Sign-in required"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::InternalError))]
pub(crate) async fn find(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiPath((_vault_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<SavedView>> {
    let access = access(&state, vault, uploader).await?;
    let view = visible(&state, vault, &access, id).await?;
    validate_references(&state, vault, &view.query).await?;
    Ok(Json(view))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSavedView {
    #[serde(default, deserialize_with = "provided")]
    #[schema(value_type = String, required = false)]
    name: Option<String>,
    #[serde(default, deserialize_with = "provided")]
    #[schema(value_type = ViewVisibility, required = false)]
    visibility: Option<ViewVisibility>,
    #[serde(default, deserialize_with = "provided")]
    #[schema(value_type = SavedViewQuery, required = false)]
    query: Option<SavedViewQuery>,
    #[serde(default, deserialize_with = "provided")]
    #[schema(value_type = ViewPresentation, required = false)]
    presentation: Option<ViewPresentation>,
}

fn provided<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[utoipa::path(patch, path = "/api/vaults/{vault_id}/saved-views/{id}", operation_id = "updateSavedView", tag = "saved-views",
    params(("vault_id" = Uuid, Path), ("id" = Uuid, Path)), request_body = UpdateSavedView,
    responses((status = 200, description = "Updated saved view", body = SavedView), (status = 401, description = "Sign-in required"), (status = 403, description = "Editing is not allowed"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn update(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiPath((_vault_id, id)): ApiPath<(Uuid, Uuid)>,
    ApiJson(patch): ApiJson<UpdateSavedView>,
) -> ApiResult<Json<SavedView>> {
    let access = access(&state, vault, uploader).await?;
    let view = visible(&state, vault, &access, id).await?;
    may_edit(&view, &access)?;
    let input = SavedViewInput {
        name: patch.name.unwrap_or(view.name),
        visibility: patch.visibility.unwrap_or(view.visibility),
        query: patch.query.unwrap_or(view.query),
        presentation: patch.presentation.unwrap_or(view.presentation),
    };
    validate(&state, vault, &input, &access).await?;
    Ok(Json(
        state.saved_views.update(vault, &access, id, input).await?,
    ))
}

#[utoipa::path(delete, path = "/api/vaults/{vault_id}/saved-views/{id}", operation_id = "deleteSavedView", tag = "saved-views",
    params(("vault_id" = Uuid, Path), ("id" = Uuid, Path)),
    responses((status = 204, description = "Deleted saved view"), (status = 401, description = "Sign-in required"), (status = 403, description = "Deletion is not allowed"), crate::openapi::NotFound, crate::openapi::InternalError))]
pub(crate) async fn delete(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiPath((_vault_id, id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let access = access(&state, vault, uploader).await?;
    let view = visible(&state, vault, &access, id).await?;
    may_edit(&view, &access)?;
    state.saved_views.delete(vault, &access, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
