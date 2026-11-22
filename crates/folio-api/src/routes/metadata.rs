use axum::{
    extract::{FromRequest, Request, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use folio_core::models::{MetadataField, MetadataFieldInput, MetadataOption, MetadataOptionInput};
use serde::{de::DeserializeOwned, Deserialize};
use utoipa::IntoParams;
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    extract::{ApiPath, ApiQuery},
    state::AppState,
    vault::DefaultVault,
};

pub(crate) struct MetadataJson<T>(T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for MetadataJson<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(input) = Json::<T>::from_request(request, state)
            .await
            .map_err(|error| {
                if error.status() == StatusCode::UNPROCESSABLE_ENTITY {
                    ApiError::bad_request(error.body_text())
                } else {
                    error.into()
                }
            })?;
        Ok(Self(input))
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/vaults/{vault_id}/metadata-fields",
            get(list_fields).post(create_field),
        )
        .route(
            "/vaults/{vault_id}/metadata-fields/{field_id}",
            get(get_field).patch(update_field).delete(delete_field),
        )
        .route(
            "/vaults/{vault_id}/metadata-fields/{field_id}/restore",
            post(restore_field),
        )
        .route(
            "/vaults/{vault_id}/metadata-fields/{field_id}/options",
            get(list_options).post(create_option),
        )
        .route(
            "/vaults/{vault_id}/metadata-fields/{field_id}/options/{option_id}",
            get(get_option).patch(update_option).delete(delete_option),
        )
        .route(
            "/vaults/{vault_id}/metadata-fields/{field_id}/options/{option_id}/restore",
            post(restore_option),
        )
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct ListMetadataQuery {
    #[serde(default)]
    include_deleted: bool,
    after: Option<Uuid>,
    #[serde(default = "page_size")]
    limit: u32,
}
fn page_size() -> u32 {
    50
}
impl ListMetadataQuery {
    fn validate(&self) -> ApiResult<()> {
        if !(1..=100).contains(&self.limit) {
            return Err(ApiError::bad_request("limit must be between 1 and 100"));
        }
        Ok(())
    }
}

#[utoipa::path(get, path = "/api/vaults/{vault_id}/metadata-fields", operation_id = "listMetadataFields", tag = "metadata", params(("vault_id" = Uuid, Path), ListMetadataQuery), responses((status = 200, description = "Definitions in stable ID order; pass the last ID as after for the next page", body = Vec<MetadataField>), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn list_fields(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id,)): ApiPath<(Uuid,)>,
    ApiQuery(query): ApiQuery<ListMetadataQuery>,
) -> ApiResult<Json<Vec<MetadataField>>> {
    query.validate()?;
    Ok(Json(
        state
            .metadata_fields
            .list(vault, query.include_deleted, query.after, query.limit)
            .await?,
    ))
}

#[utoipa::path(get, path = "/api/vaults/{vault_id}/metadata-fields/{field_id}", operation_id = "getMetadataField", tag = "metadata", params(("vault_id" = Uuid, Path), ("field_id" = Uuid, Path)), responses((status = 200, description = "Metadata field", body = MetadataField), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn get_field(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id, field_id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<MetadataField>> {
    Ok(Json(
        state
            .metadata_fields
            .find(vault, field_id)
            .await?
            .ok_or_else(|| ApiError::not_found("Metadata field not found"))?,
    ))
}

#[utoipa::path(post, path = "/api/vaults/{vault_id}/metadata-fields", operation_id = "createMetadataField", tag = "metadata", params(("vault_id" = Uuid, Path)), request_body = MetadataFieldInput, responses((status = 201, description = "Metadata field", body = MetadataField), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn create_field(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id,)): ApiPath<(Uuid,)>,
    MetadataJson(input): MetadataJson<MetadataFieldInput>,
) -> ApiResult<(StatusCode, Json<MetadataField>)> {
    Ok((
        StatusCode::CREATED,
        Json(state.metadata_fields.create(vault, input).await?),
    ))
}

#[utoipa::path(patch, path = "/api/vaults/{vault_id}/metadata-fields/{field_id}", operation_id = "updateMetadataField", description = "Replace editable properties. Name and kind are required; omitted required and unique flags become false.", tag = "metadata", params(("vault_id" = Uuid, Path), ("field_id" = Uuid, Path)), request_body = MetadataFieldInput, responses((status = 200, description = "Metadata field", body = MetadataField), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn update_field(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id, field_id)): ApiPath<(Uuid, Uuid)>,
    MetadataJson(input): MetadataJson<MetadataFieldInput>,
) -> ApiResult<Json<MetadataField>> {
    Ok(Json(
        state.metadata_fields.update(vault, field_id, input).await?,
    ))
}

#[utoipa::path(delete, path = "/api/vaults/{vault_id}/metadata-fields/{field_id}", operation_id = "deleteMetadataField", tag = "metadata", params(("vault_id" = Uuid, Path), ("field_id" = Uuid, Path)), responses((status = 204, description = "Metadata field"), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn delete_field(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id, field_id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    state
        .metadata_fields
        .set_deleted(vault, field_id, true)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/vaults/{vault_id}/metadata-fields/{field_id}/restore", operation_id = "restoreMetadataField", tag = "metadata", params(("vault_id" = Uuid, Path), ("field_id" = Uuid, Path)), responses((status = 200, description = "Metadata field", body = MetadataField), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn restore_field(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id, field_id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<MetadataField>> {
    Ok(Json(
        state
            .metadata_fields
            .set_deleted(vault, field_id, false)
            .await?,
    ))
}

#[utoipa::path(get, path = "/api/vaults/{vault_id}/metadata-fields/{field_id}/options", operation_id = "listMetadataOptions", tag = "metadata", params(("vault_id" = Uuid, Path), ("field_id" = Uuid, Path), ListMetadataQuery), responses((status = 200, description = "Options in stable ID order; pass the last ID as after for the next page", body = Vec<MetadataOption>), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn list_options(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id, field_id)): ApiPath<(Uuid, Uuid)>,
    ApiQuery(query): ApiQuery<ListMetadataQuery>,
) -> ApiResult<Json<Vec<MetadataOption>>> {
    query.validate()?;
    state
        .metadata_fields
        .find(vault, field_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Metadata field not found"))?;
    Ok(Json(
        state
            .metadata_options
            .list(
                vault,
                field_id,
                query.include_deleted,
                query.after,
                query.limit,
            )
            .await?,
    ))
}

#[utoipa::path(get, path = "/api/vaults/{vault_id}/metadata-fields/{field_id}/options/{option_id}", operation_id = "getMetadataOption", tag = "metadata", params(("vault_id" = Uuid, Path), ("field_id" = Uuid, Path), ("option_id" = Uuid, Path)), responses((status = 200, description = "Metadata option", body = MetadataOption), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn get_option(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id, field_id, option_id)): ApiPath<(Uuid, Uuid, Uuid)>,
) -> ApiResult<Json<MetadataOption>> {
    Ok(Json(
        state
            .metadata_options
            .find(vault, field_id, option_id)
            .await?
            .ok_or_else(|| ApiError::not_found("Metadata option not found"))?,
    ))
}

#[utoipa::path(post, path = "/api/vaults/{vault_id}/metadata-fields/{field_id}/options", operation_id = "createMetadataOption", tag = "metadata", params(("vault_id" = Uuid, Path), ("field_id" = Uuid, Path)), request_body = MetadataOptionInput, responses((status = 201, description = "Metadata option", body = MetadataOption), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn create_option(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id, field_id)): ApiPath<(Uuid, Uuid)>,
    MetadataJson(input): MetadataJson<MetadataOptionInput>,
) -> ApiResult<(StatusCode, Json<MetadataOption>)> {
    Ok((
        StatusCode::CREATED,
        Json(
            state
                .metadata_options
                .create(vault, field_id, input)
                .await?,
        ),
    ))
}

#[utoipa::path(patch, path = "/api/vaults/{vault_id}/metadata-fields/{field_id}/options/{option_id}", operation_id = "updateMetadataOption", description = "Replace label and position. Label is required; omitted position becomes zero.", tag = "metadata", params(("vault_id" = Uuid, Path), ("field_id" = Uuid, Path), ("option_id" = Uuid, Path)), request_body = MetadataOptionInput, responses((status = 200, description = "Metadata option", body = MetadataOption), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn update_option(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id, field_id, option_id)): ApiPath<(Uuid, Uuid, Uuid)>,
    MetadataJson(input): MetadataJson<MetadataOptionInput>,
) -> ApiResult<Json<MetadataOption>> {
    Ok(Json(
        state
            .metadata_options
            .update(vault, field_id, option_id, input)
            .await?,
    ))
}

#[utoipa::path(delete, path = "/api/vaults/{vault_id}/metadata-fields/{field_id}/options/{option_id}", operation_id = "deleteMetadataOption", tag = "metadata", params(("vault_id" = Uuid, Path), ("field_id" = Uuid, Path), ("option_id" = Uuid, Path)), responses((status = 204, description = "Metadata option"), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn delete_option(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id, field_id, option_id)): ApiPath<(Uuid, Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    state
        .metadata_options
        .set_deleted(vault, field_id, option_id, true)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/vaults/{vault_id}/metadata-fields/{field_id}/options/{option_id}/restore", operation_id = "restoreMetadataOption", tag = "metadata", params(("vault_id" = Uuid, Path), ("field_id" = Uuid, Path), ("option_id" = Uuid, Path)), responses((status = 200, description = "Metadata option", body = MetadataOption), (status = 401, description = "Authentication required when enabled"), (status = 403, description = "Mutations require an Editor or Owner vault role and write scope"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn restore_option(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((_vault_id, field_id, option_id)): ApiPath<(Uuid, Uuid, Uuid)>,
) -> ApiResult<Json<MetadataOption>> {
    Ok(Json(
        state
            .metadata_options
            .set_deleted(vault, field_id, option_id, false)
            .await?,
    ))
}
