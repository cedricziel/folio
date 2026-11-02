use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use folio_core::{
    models::{DocumentEntry, EntryBody, EntryKind, EntryPageQuery, EntryPermissions, VaultRole},
    ports::EntryCreation,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    error::{ApiError, ApiResult},
    extract::{ApiJson, ApiPath, ApiQuery},
    state::AppState,
    vault::{DefaultVault, VaultContext, CURRENT_VAULT},
};

#[derive(Serialize, ToSchema)]
pub struct EntryView {
    #[serde(flatten)]
    entry: DocumentEntry,
    #[serde(flatten)]
    permissions: EntryPermissions,
}

impl EntryView {
    fn new(entry: DocumentEntry, context: &VaultContext) -> Self {
        let permissions = context.user.as_ref().map_or(
            EntryPermissions {
                can_edit: false,
                can_delete: context.role >= VaultRole::Editor,
            },
            |user| EntryPermissions::for_member(context.role, user, entry.created_by.as_ref()),
        );
        Self { entry, permissions }
    }
}

#[derive(Serialize, ToSchema)]
pub struct EntryPageView {
    entries: Vec<EntryView>,
    next_cursor: Option<String>,
    can_create: bool,
}

#[derive(Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct EntryQuery {
    kind: EntryKind,
    /// Page size from 1 to 100; defaults to 50.
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateEntryRequest {
    kind: EntryKind,
    /// Nonblank plain text, at most 16384 UTF-8 bytes.
    body: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateEntryRequest {
    /// Nonblank plain text, at most 16384 UTF-8 bytes.
    body: String,
}

fn context() -> ApiResult<VaultContext> {
    CURRENT_VAULT
        .try_with(Clone::clone)
        .map_err(|_| ApiError::new(StatusCode::UNAUTHORIZED, "Vault context required"))
}

fn writable(context: &VaultContext) -> ApiResult<()> {
    if context.role < VaultRole::Contributor {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "This vault role cannot add or change entries",
        ));
    }
    Ok(())
}

#[utoipa::path(get, path = "/api/documents/{id}/entries", operation_id = "listDocumentEntries", tag = "documents",
    params(("id" = Uuid, Path), EntryQuery), responses((status = 200, body = EntryPageView), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::InternalError))]
pub(crate) async fn list(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(document): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<EntryQuery>,
) -> ApiResult<Json<EntryPageView>> {
    let context = context()?;
    let cursor = query
        .cursor
        .map(|value| {
            let bytes = URL_SAFE_NO_PAD
                .decode(value)
                .map_err(|_| ApiError::bad_request("Invalid entry cursor"))?;
            serde_json::from_slice(&bytes)
                .map_err(|_| ApiError::bad_request("Invalid entry cursor"))
        })
        .transpose()?;
    let page = state
        .entry_repo
        .page(
            vault,
            document,
            EntryPageQuery {
                kind: query.kind,
                limit: query.limit.unwrap_or(50),
                cursor,
            },
        )
        .await?;
    let next_cursor = page
        .next_cursor
        .map(|cursor| serde_json::to_vec(&cursor).map(|bytes| URL_SAFE_NO_PAD.encode(bytes)))
        .transpose()
        .map_err(|_| ApiError::internal("Could not encode entry cursor"))?;
    Ok(Json(EntryPageView {
        entries: page
            .entries
            .into_iter()
            .map(|entry| EntryView::new(entry, &context))
            .collect(),
        next_cursor,
        can_create: context.role >= VaultRole::Contributor,
    }))
}

#[utoipa::path(post, path = "/api/documents/{id}/entries", operation_id = "createDocumentEntry", tag = "documents",
    params(("id" = Uuid, Path), ("Idempotency-Key" = Uuid, Header, description = "Required nonnil UUID retained across retries")), request_body = CreateEntryRequest,
    responses((status = 201, body = EntryView), (status = 403, description = "Read-only membership"), (status = 410, description = "The original entry was deleted"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn create(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(document): ApiPath<Uuid>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<CreateEntryRequest>,
) -> ApiResult<(StatusCode, Json<EntryView>)> {
    let context = context()?;
    writable(&context)?;
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<Uuid>().ok())
        .filter(|key| !key.is_nil())
        .ok_or_else(|| ApiError::bad_request("Idempotency-Key must be a nonnil UUID"))?;
    let result = state
        .entry_repo
        .create(
            vault,
            document,
            request.kind,
            EntryBody::parse(request.body)?,
            key,
        )
        .await?;
    let entry = match result {
        EntryCreation::Created(entry) | EntryCreation::Replayed(entry) => entry,
        EntryCreation::Deleted => {
            return Err(ApiError::new(
                StatusCode::GONE,
                "The original entry was deleted",
            ))
        }
    };
    Ok((StatusCode::CREATED, Json(EntryView::new(entry, &context))))
}

#[utoipa::path(patch, path = "/api/documents/{id}/entries/{entry_id}", operation_id = "updateDocumentEntry", tag = "documents",
    params(("id" = Uuid, Path), ("entry_id" = Uuid, Path)), request_body = UpdateEntryRequest,
    responses((status = 200, body = EntryView), (status = 403, description = "Only a writable author may edit"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::InternalError))]
pub(crate) async fn update(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((document, entry)): ApiPath<(Uuid, Uuid)>,
    ApiJson(request): ApiJson<UpdateEntryRequest>,
) -> ApiResult<Json<EntryView>> {
    let context = context()?;
    writable(&context)?;
    let current = state
        .entry_repo
        .find(vault, document, entry)
        .await?
        .ok_or_else(|| ApiError::not_found("Entry not found"))?;
    if !EntryView::new(current, &context).permissions.can_edit {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Only the author may edit this entry",
        ));
    }
    let entry = state
        .entry_repo
        .update(vault, document, entry, EntryBody::parse(request.body)?)
        .await?;
    Ok(Json(EntryView::new(entry, &context)))
}

#[utoipa::path(delete, path = "/api/documents/{id}/entries/{entry_id}", operation_id = "deleteDocumentEntry", tag = "documents",
    params(("id" = Uuid, Path), ("entry_id" = Uuid, Path)),
    responses((status = 204, description = "Entry deleted"), (status = 403, description = "Requires author or moderation permission"), crate::openapi::NotFound, crate::openapi::InternalError))]
pub(crate) async fn delete(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((document, entry)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    let context = context()?;
    writable(&context)?;
    if let Some(current) = state.entry_repo.find(vault, document, entry).await? {
        if !EntryView::new(current, &context).permissions.can_delete {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "This entry requires moderation permission",
            ));
        }
    }
    state
        .entry_repo
        .delete(vault, document, entry, context.role >= VaultRole::Editor)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
