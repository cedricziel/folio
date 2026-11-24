use askama::Template;
use axum::{
    extract::State,
    response::{Html, Response},
    Json,
};
use chrono::{DateTime, Utc};
use folio_core::models::{
    Access, DocumentRevision, DocumentRevisionPage, OriginalVerification, VaultAction, VaultId,
};
use serde::{Deserialize, Serialize};
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
struct RevisionView {
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
    require_read_access(&state, default, vault, uploader).await?;
    list_in_vault(&state, vault, id, query).await
}

async fn require_read_access(
    state: &AppState,
    default: VaultId,
    vault: VaultId,
    uploader: Uploader,
) -> ApiResult<()> {
    let allowed = match uploader.0 {
        Some(user) => matches!(
            state.access.can(&user, vault, VaultAction::Read).await?,
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
    }))
}

#[derive(Template)]
#[template(path = "partials/document_file_history.html")]
struct FileHistoryTemplate {
    document_id: Uuid,
    revisions: Vec<DocumentRevision>,
    next_before: Option<i64>,
    show_latest: bool,
}

pub(super) async fn panel(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<RevisionQuery>,
) -> ApiResult<Html<String>> {
    let show_latest = query.before.is_some();
    let DocumentRevisionPage {
        revisions,
        next_before,
    } = revision_page(&state, vault, id, query).await?;
    FileHistoryTemplate {
        document_id: id,
        revisions,
        next_before,
        show_latest,
    }
    .render()
    .map(Html)
    .map_err(|_| ApiError::internal("Could not render file history"))
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
    require_read_access(&state, default, vault, uploader).await?;
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
