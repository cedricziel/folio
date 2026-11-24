use askama::Template;
use axum::{
    extract::{multipart::MultipartRejection, rejection::JsonRejection, Multipart, State},
    http::{HeaderMap, StatusCode},
    response::{Html, Response},
    Json,
};
use chrono::{DateTime, Utc};
use folio_core::models::{
    Access, DocumentRevision, DocumentRevisionPage, OriginalVerification, RevisionPublication,
    RevisionPublicationResult, RevisionPublicationSource, VaultAction, VaultId,
};
use folio_core::mutation::{current_actor, current_lineage, with_actor, with_lineage};
use folio_core::services::{replace_document_original, NewRevisionOriginal};
use folio_infra::queue::REVISION_DISPATCH_ENABLED;
use serde::{Deserialize, Serialize};
use tracing::Instrument;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::documents::FileContent;
use crate::{
    error::{ApiError, ApiResult},
    extract::{ApiPath, ApiQuery},
    state::AppState,
    vault::{filters, DefaultVault, Uploader},
};

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum RevisionVerification {
    Pending,
    Verified,
    Unavailable,
}

#[derive(Serialize, ToSchema)]
struct RevisionOriginalView {
    filename: Option<String>,
    mime_type: String,
    verification: RevisionVerification,
    sha256: Option<String>,
    byte_size: Option<u64>,
}

#[derive(Serialize, ToSchema)]
pub(crate) struct RevisionView {
    id: Uuid,
    number: i64,
    is_current: bool,
    created_at: DateTime<Utc>,
    created_by: Option<String>,
    restored_from_revision_id: Option<Uuid>,
    original: RevisionOriginalView,
}

impl From<DocumentRevision> for RevisionView {
    fn from(revision: DocumentRevision) -> Self {
        let original = revision.original;
        let verification = match original.verification {
            OriginalVerification::Pending => RevisionVerification::Pending,
            OriginalVerification::Verified => RevisionVerification::Verified,
            OriginalVerification::Unavailable => RevisionVerification::Unavailable,
        };
        Self {
            id: revision.id,
            number: revision.number,
            is_current: revision.is_current,
            created_at: revision.created_at,
            created_by: revision.created_by.map(|user| user.0),
            restored_from_revision_id: revision.restored_from_revision_id,
            original: RevisionOriginalView {
                filename: original.filename,
                mime_type: original.mime_type,
                verification,
                sha256: original.sha256,
                byte_size: original.byte_size,
            },
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct RevisionPageView {
    revisions: Vec<RevisionView>,
    next_before: Option<i64>,
    /// This server is accepting replacement and restoration requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = bool, required = false)]
    publication_enabled: Option<bool>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RevisionQuery {
    /// Exclusive upper bound on revision number.
    before: Option<i64>,
    /// Page size from 1 to 100. Defaults to 25.
    limit: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/api/documents/{id}/revisions",
    operation_id = "listDocumentRevisions",
    tag = "documents",
    params(("id" = Uuid, Path), RevisionQuery),
    responses(
        (status = 200, description = "Revision history, newest first", body = RevisionPageView),
        crate::openapi::BadRequest,
        crate::openapi::NotFound,
        crate::openapi::InternalError,
    )
)]
pub(crate) async fn list(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<RevisionQuery>,
) -> ApiResult<Json<RevisionPageView>> {
    list_in_vault(&state, vault, id, query).await
}

#[utoipa::path(
    get,
    path = "/api/vaults/{vault_id}/documents/{id}/revisions",
    operation_id = "listVaultDocumentRevisions",
    tag = "documents",
    params(("vault_id" = Uuid, Path), ("id" = Uuid, Path), RevisionQuery),
    responses(
        (status = 200, description = "Revision history in the requested vault", body = RevisionPageView),
        crate::openapi::BadRequest,
        crate::openapi::NotFound,
        crate::openapi::InternalError,
    )
)]
pub(crate) async fn list_scoped(
    DefaultVault(default): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiPath((vault, id)): ApiPath<(Uuid, Uuid)>,
    ApiQuery(query): ApiQuery<RevisionQuery>,
) -> ApiResult<Json<RevisionPageView>> {
    let vault = VaultId(vault);
    require_read_access(&state, default, vault, &uploader).await?;
    list_in_vault(&state, vault, id, query).await
}

async fn require_read_access(
    state: &AppState,
    default: VaultId,
    vault: VaultId,
    uploader: &Uploader,
) -> ApiResult<()> {
    let allowed = match uploader.0.as_ref() {
        Some(user) => matches!(
            state.access.can(user, vault, VaultAction::Read).await?,
            Access::Allowed(_)
        ),
        None => default == vault,
    };
    if !allowed {
        return Err(ApiError::not_found("Vault not found"));
    }
    Ok(())
}

async fn list_in_vault(
    state: &AppState,
    vault: VaultId,
    id: Uuid,
    query: RevisionQuery,
) -> ApiResult<Json<RevisionPageView>> {
    let DocumentRevisionPage {
        revisions,
        next_before,
    } = revision_page(state, vault, id, query).await?;
    Ok(Json(RevisionPageView {
        revisions: revisions.into_iter().map(RevisionView::from).collect(),
        next_before,
        publication_enabled: REVISION_DISPATCH_ENABLED.then_some(true),
    }))
}

#[derive(Template)]
#[template(path = "partials/document_file_history.html")]
struct FileHistoryTemplate {
    document_id: Uuid,
    revisions: Vec<DocumentRevision>,
    next_before: Option<i64>,
    show_latest: bool,
    editable_current_revision: Option<Uuid>,
}

pub(super) async fn panel(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<RevisionQuery>,
) -> ApiResult<Html<String>> {
    let show_latest = query.before.is_some();
    let DocumentRevisionPage {
        revisions,
        next_before,
    } = revision_page(&state, vault, id, query).await?;
    let editable_current_revision = if REVISION_DISPATCH_ENABLED {
        let current = if show_latest {
            state
                .document_repo
                .list_revisions(vault, id, None, 1)
                .await?
        } else {
            Vec::new()
        };
        let latest = if show_latest {
            current.first()
        } else {
            revisions.first()
        };
        editable_revision(&state, vault, id, &uploader, latest).await?
    } else {
        None
    };
    FileHistoryTemplate {
        document_id: id,
        revisions,
        next_before,
        show_latest,
        editable_current_revision,
    }
    .render()
    .map(Html)
    .map_err(|_| ApiError::internal("Could not render file history"))
}

async fn editable_revision(
    state: &AppState,
    vault: VaultId,
    id: Uuid,
    uploader: &Uploader,
    current: Option<&DocumentRevision>,
) -> ApiResult<Option<Uuid>> {
    let Some(current) = current.filter(|revision| {
        revision.is_current && revision.original.verification == OriginalVerification::Verified
    }) else {
        return Ok(None);
    };
    if let Some(user) = &uploader.0 {
        let document = state
            .document_repo
            .find_by_id(vault, id)
            .await?
            .ok_or_else(|| ApiError::not_found("Document not found"))?;
        let action = VaultAction::EditDocument {
            uploaded_by: document.uploaded_by,
        };
        if !matches!(
            state.access.can(user, vault, action).await?,
            Access::Allowed(_)
        ) {
            return Ok(None);
        }
    }
    Ok(Some(current.id))
}

async fn revision_page(
    state: &AppState,
    vault: VaultId,
    id: Uuid,
    query: RevisionQuery,
) -> ApiResult<DocumentRevisionPage> {
    state
        .document_repo
        .revision_page(vault, id, query.before, query.limit)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))
}

#[utoipa::path(
    get,
    path = "/api/documents/{id}/revisions/{revision_id}/download",
    operation_id = "downloadDocumentRevision",
    tag = "documents",
    params(("id" = Uuid, Path), ("revision_id" = Uuid, Path)),
    responses(
        (status = 200, description = "Original bytes of this revision", content_type = "application/octet-stream", body = FileContent),
        crate::openapi::BadRequest,
        crate::openapi::NotFound,
        crate::openapi::InternalError,
    )
)]
pub(crate) async fn download(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath((id, revision_id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Response> {
    download_in_vault(&state, vault, id, revision_id).await
}

#[utoipa::path(
    get,
    path = "/api/vaults/{vault_id}/documents/{id}/revisions/{revision_id}/download",
    operation_id = "downloadVaultDocumentRevision",
    tag = "documents",
    params(("vault_id" = Uuid, Path), ("id" = Uuid, Path), ("revision_id" = Uuid, Path)),
    responses(
        (status = 200, description = "Original bytes of this revision in the requested vault",
            content_type = "application/octet-stream", body = FileContent),
        crate::openapi::BadRequest,
        crate::openapi::NotFound,
        crate::openapi::InternalError,
    )
)]
pub(crate) async fn download_scoped(
    DefaultVault(default): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiPath((vault, id, revision_id)): ApiPath<(Uuid, Uuid, Uuid)>,
) -> ApiResult<Response> {
    let vault = VaultId(vault);
    require_read_access(&state, default, vault, &uploader).await?;
    download_in_vault(&state, vault, id, revision_id).await
}

async fn download_in_vault(
    state: &AppState,
    vault: VaultId,
    id: Uuid,
    revision_id: Uuid,
) -> ApiResult<Response> {
    let original = state
        .document_repo
        .find_revision(vault, id, revision_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document revision not found"))?
        .original;
    let filename = original
        .filename
        .unwrap_or_else(|| format!("document-{id}"));
    super::documents::download_original(
        state,
        &original.storage_key,
        &original.mime_type,
        &filename,
    )
    .await
}

#[utoipa::path(
    post,
    path = "/api/documents/{id}/revisions",
    operation_id = "replaceDocumentRevision",
    tag = "documents",
    params(("id" = Uuid, Path), ("Idempotency-Key" = Uuid, Header, description = "A nonnil UUID reused across retries of one request.")),
    request_body(content = ReplaceRevisionRequest, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "The revision previously published for this request", body = RevisionView),
        (status = 201, description = "The new current revision", body = RevisionView),
        crate::openapi::BadRequest,
        (status = 401, description = "Missing or invalid authentication"),
        (status = 403, description = "Document editing is not permitted", body = crate::error::ErrorResponse),
        crate::openapi::NotFound,
        (status = 409, description = "The expected revision is stale, the original cannot be changed, or the request key conflicts", body = crate::error::ErrorResponse),
        crate::openapi::PayloadTooLarge,
        crate::openapi::InternalError,
    ),
)]
pub(crate) async fn replace(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<(StatusCode, Json<RevisionView>)> {
    authorize_revision_edit(&state, vault, vault, &uploader, id).await?;
    replace_in_vault(&state, vault, id, &headers, multipart).await
}

#[utoipa::path(
    post,
    path = "/api/vaults/{vault_id}/documents/{id}/revisions",
    operation_id = "replaceVaultDocumentRevision",
    tag = "documents",
    params(("vault_id" = Uuid, Path), ("id" = Uuid, Path), ("Idempotency-Key" = Uuid, Header, description = "A nonnil UUID reused across retries of one request.")),
    request_body(content = ReplaceRevisionRequest, content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "The revision previously published for this request", body = RevisionView),
        (status = 201, description = "The new current revision", body = RevisionView),
        crate::openapi::BadRequest,
        (status = 401, description = "Missing or invalid authentication"),
        (status = 403, description = "Document editing is not permitted", body = crate::error::ErrorResponse),
        crate::openapi::NotFound,
        (status = 409, description = "The expected revision is stale, the original cannot be changed, or the request key conflicts", body = crate::error::ErrorResponse),
        crate::openapi::PayloadTooLarge,
        crate::openapi::InternalError,
    ),
)]
pub(crate) async fn replace_scoped(
    DefaultVault(default): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiPath((vault, id)): ApiPath<(Uuid, Uuid)>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<(StatusCode, Json<RevisionView>)> {
    let vault = VaultId(vault);
    authorize_revision_edit(&state, default, vault, &uploader, id).await?;
    replace_in_vault(&state, vault, id, &headers, multipart).await
}

#[utoipa::path(
    post,
    path = "/api/documents/{id}/revisions/{revision_id}/restore",
    operation_id = "restoreDocumentRevision",
    tag = "documents",
    params(("id" = Uuid, Path), ("revision_id" = Uuid, Path), ("Idempotency-Key" = Uuid, Header, description = "A nonnil UUID reused across retries of one request.")),
    request_body(content = RestoreRevisionRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "The revision previously published for this request", body = RevisionView),
        (status = 201, description = "The new current revision", body = RevisionView),
        crate::openapi::BadRequest,
        (status = 401, description = "Missing or invalid authentication"),
        (status = 403, description = "Document editing is not permitted", body = crate::error::ErrorResponse),
        crate::openapi::NotFound,
        (status = 409, description = "The expected revision is stale, the original cannot be changed, or the request key conflicts", body = crate::error::ErrorResponse),
        crate::openapi::PayloadTooLarge,
        crate::openapi::InternalError,
    ),
)]
pub(crate) async fn restore(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiPath((id, revision_id)): ApiPath<(Uuid, Uuid)>,
    headers: HeaderMap,
    request: Result<Json<RestoreRevisionRequest>, JsonRejection>,
) -> ApiResult<(StatusCode, Json<RevisionView>)> {
    authorize_revision_edit(&state, vault, vault, &uploader, id).await?;
    restore_in_vault(&state, vault, id, revision_id, &headers, request).await
}

#[utoipa::path(
    post,
    path = "/api/vaults/{vault_id}/documents/{id}/revisions/{revision_id}/restore",
    operation_id = "restoreVaultDocumentRevision",
    tag = "documents",
    params(("vault_id" = Uuid, Path), ("id" = Uuid, Path), ("revision_id" = Uuid, Path), ("Idempotency-Key" = Uuid, Header, description = "A nonnil UUID reused across retries of one request.")),
    request_body(content = RestoreRevisionRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "The revision previously published for this request", body = RevisionView),
        (status = 201, description = "The new current revision", body = RevisionView),
        crate::openapi::BadRequest,
        (status = 401, description = "Missing or invalid authentication"),
        (status = 403, description = "Document editing is not permitted", body = crate::error::ErrorResponse),
        crate::openapi::NotFound,
        (status = 409, description = "The expected revision is stale, the original cannot be changed, or the request key conflicts", body = crate::error::ErrorResponse),
        crate::openapi::PayloadTooLarge,
        crate::openapi::InternalError,
    ),
)]
pub(crate) async fn restore_scoped(
    DefaultVault(default): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiPath((vault, id, revision_id)): ApiPath<(Uuid, Uuid, Uuid)>,
    headers: HeaderMap,
    request: Result<Json<RestoreRevisionRequest>, JsonRejection>,
) -> ApiResult<(StatusCode, Json<RevisionView>)> {
    let vault = VaultId(vault);
    authorize_revision_edit(&state, default, vault, &uploader, id).await?;
    restore_in_vault(&state, vault, id, revision_id, &headers, request).await
}

#[derive(ToSchema)]
#[allow(dead_code)]
pub(crate) struct ReplaceRevisionRequest {
    expected_current_revision_id: Uuid,
    #[schema(value_type = String, format = Binary)]
    file: Vec<u8>,
}

#[derive(Deserialize, ToSchema)]
pub(crate) struct RestoreRevisionRequest {
    expected_current_revision_id: Uuid,
}

async fn authorize_revision_edit(
    state: &AppState,
    default: VaultId,
    vault: VaultId,
    uploader: &Uploader,
    id: Uuid,
) -> ApiResult<()> {
    require_read_access(state, default, vault, uploader).await?;
    let document = state
        .document_repo
        .find_by_id(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document not found"))?;
    super::trash::authorize(
        state,
        vault,
        uploader,
        VaultAction::EditDocument {
            uploaded_by: document.uploaded_by,
        },
    )
    .await
}

fn publication_key(headers: &HeaderMap) -> ApiResult<Uuid> {
    if headers.get_all("idempotency-key").iter().count() != 1 {
        return Err(ApiError::bad_request("Provide exactly one Idempotency-Key"));
    }
    super::pages::idempotency_key(headers)?
        .ok_or_else(|| ApiError::bad_request("Idempotency-Key is required"))
}

async fn replace_in_vault(
    state: &AppState,
    vault: VaultId,
    id: Uuid,
    headers: &HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<(StatusCode, Json<RevisionView>)> {
    let key = publication_key(headers)?;
    let mut multipart = multipart.map_err(|e| ApiError::new(e.status(), e.body_text()))?;
    let mut file = None;
    let mut expected_current = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| super::pages::multipart_error("Invalid revision form", e))?
    {
        match field.name() {
            Some("expected_current_revision_id") if expected_current.is_none() => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| super::pages::multipart_error("Invalid current revision", e))?;
                expected_current = Some(
                    Uuid::parse_str(value.trim())
                        .ok()
                        .filter(|revision| !revision.is_nil())
                        .ok_or_else(|| {
                            ApiError::bad_request(
                                "The expected current revision must be a nonnil UUID",
                            )
                        })?,
                );
            }
            Some("file") if file.is_none() => {
                let filename = field
                    .file_name()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("upload")
                    .to_owned();
                let mime_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_owned();
                let bytes = field.bytes().await.map_err(|e| {
                    super::pages::multipart_error("Failed to read replacement file", e)
                })?;
                file = Some((filename, mime_type, bytes));
            }
            _ => {
                return Err(ApiError::bad_request(
                    "Provide one file and one expected current revision",
                ))
            }
        }
    }
    let expected_current = expected_current
        .ok_or_else(|| ApiError::bad_request("The expected current revision is required"))?;
    let (filename, mime_type, bytes) =
        file.ok_or_else(|| ApiError::bad_request("A replacement file is required"))?;
    let upload = NewRevisionOriginal {
        key,
        expected_current,
        filename,
        mime_type,
        bytes,
    };

    let documents = state.document_repo.clone();
    let store = state.store.clone();
    let result = state
        .upload_tasks
        .clone()
        .spawn(
            with_actor(
                current_actor(),
                with_lineage(current_lineage(), async move {
                    replace_document_original(documents.as_ref(), store.as_ref(), vault, id, upload)
                        .await
                }),
            )
            .instrument(tracing::Span::current()),
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, "Replacement upload task failed");
            ApiError::internal("Replacement upload failed")
        })??;
    publication_response(state, vault, id, result).await
}

async fn restore_in_vault(
    state: &AppState,
    vault: VaultId,
    id: Uuid,
    revision: Uuid,
    headers: &HeaderMap,
    request: Result<Json<RestoreRevisionRequest>, JsonRejection>,
) -> ApiResult<(StatusCode, Json<RevisionView>)> {
    let key = publication_key(headers)?;
    let Json(request) = request.map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::new(e.status(), e.body_text())
        } else {
            ApiError::bad_request("Provide the expected current revision as JSON")
        }
    })?;
    let expected_current = request.expected_current_revision_id;
    if expected_current.is_nil() {
        return Err(ApiError::bad_request(
            "The expected current revision must be a nonnil UUID",
        ));
    }
    let result = state
        .document_repo
        .publish_revision(
            vault,
            id,
            &RevisionPublication {
                key,
                expected_current,
                source: RevisionPublicationSource::Restore { revision },
            },
        )
        .await?;
    publication_response(state, vault, id, result).await
}

async fn publication_response(
    state: &AppState,
    vault: VaultId,
    document: Uuid,
    result: RevisionPublicationResult,
) -> ApiResult<(StatusCode, Json<RevisionView>)> {
    let revision = state
        .document_repo
        .find_revision(vault, document, result.revision_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Document revision not found"))?;
    let status = if result.created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(revision.into())))
}
