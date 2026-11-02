use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, Redirect},
    routing::{get, post},
    Json, Router,
};
use folio_core::models::{Access, Document, VaultAction, VaultId, VaultRole};
use uuid::Uuid;

use super::view::{Nav, Sidebar};
use crate::{
    error::{ApiError, ApiResult},
    extract::ApiPath,
    state::AppState,
    vault::{filters, DefaultVault, Uploader, CURRENT_VAULT},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/trash", get(page))
        .route("/documents/{id}/restore", post(restore_page))
        .route("/documents/{id}/delete-forever", post(purge_page))
}

pub(super) async fn authorize(
    state: &AppState,
    vault: VaultId,
    user: &Uploader,
    action: VaultAction,
) -> ApiResult<()> {
    if let Some(user) = &user.0 {
        match state.access.can(user, vault, action).await? {
            Access::Allowed(_) => {}
            Access::Forbidden(_) => {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "This action is not permitted",
                ))
            }
            Access::NotVisible => return Err(ApiError::not_found("Vault not found")),
        }
    }
    Ok(())
}

pub(super) async fn trash_document(
    state: &AppState,
    vault: VaultId,
    user: &Uploader,
    id: Uuid,
) -> ApiResult<()> {
    let document = match state.document_repo.find_by_id(vault, id).await? {
        Some(document) => document,
        None => return trashed(state, vault, user, id).await.map(|_| ()),
    };
    authorize(
        state,
        vault,
        user,
        VaultAction::DeleteDocument {
            uploaded_by: document.uploaded_by,
        },
    )
    .await?;
    state.document_repo.delete(vault, id).await?;
    folio_infra::telemetry::metrics().documents_changed(-1);
    Ok(())
}

async fn trashed(
    state: &AppState,
    vault: VaultId,
    user: &Uploader,
    id: Uuid,
) -> ApiResult<Document> {
    let document = state
        .document_repo
        .find_trashed_by_id(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Trashed document not found"))?;
    authorize(
        state,
        vault,
        user,
        VaultAction::DeleteDocument {
            uploaded_by: document.uploaded_by.clone(),
        },
    )
    .await?;
    Ok(document)
}

#[utoipa::path(get, path = "/api/documents/trash", operation_id = "listTrash", tag = "documents",
    responses((status = 200, description = "Documents kept in trash, including pending permanent deletions", body = [Document]),
        (status = 403, description = "Access denied"), crate::openapi::InternalError))]
pub(crate) async fn list(
    DefaultVault(vault): DefaultVault,
    user: Uploader,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<Document>>> {
    authorize(&state, vault, &user, VaultAction::Read).await?;
    Ok(Json(state.document_repo.list_trash(vault).await?))
}

#[utoipa::path(post, path = "/api/documents/{id}/restore", operation_id = "restoreDocument", tag = "documents",
    params(("id" = Uuid, Path, description = "Trashed document ID")),
    responses((status = 200, description = "Document restored", body = Document), (status = 403, description = "Access denied"),
        crate::openapi::NotFound, crate::openapi::Conflict, crate::openapi::InternalError))]
pub(crate) async fn restore(
    DefaultVault(vault): DefaultVault,
    user: Uploader,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<Document>> {
    trashed(&state, vault, &user, id).await?;
    let document = state.document_repo.restore(vault, id).await?;
    folio_infra::telemetry::metrics().documents_changed(1);
    Ok(Json(document))
}

#[utoipa::path(delete, path = "/api/documents/{id}/purge", operation_id = "purgeDocument", tag = "documents",
    params(("id" = Uuid, Path, description = "Trashed document ID")),
    responses((status = 202, description = "Permanent deletion queued; retries continue until files and metadata are removed"),
        (status = 403, description = "Access denied"), crate::openapi::NotFound, crate::openapi::InternalError))]
pub(crate) async fn purge(
    DefaultVault(vault): DefaultVault,
    user: Uploader,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    trashed(&state, vault, &user, id).await?;
    state.document_repo.request_purge(vault, id).await?;
    Ok(StatusCode::ACCEPTED)
}

struct TrashRow {
    can_delete: bool,
    id: Uuid,
    title: String,
    subline: String,
    purge_pending: bool,
}

#[derive(Template)]
#[template(path = "trash.html")]
struct TrashTemplate {
    sidebar: Sidebar,
    rows: Vec<TrashRow>,
}

async fn page(
    DefaultVault(vault): DefaultVault,
    user: Uploader,
    State(state): State<AppState>,
) -> ApiResult<Html<String>> {
    authorize(&state, vault, &user, VaultAction::Read).await?;
    let rows = state
        .document_repo
        .list_trash(vault)
        .await?
        .into_iter()
        .map(|d| TrashRow {
            can_delete: CURRENT_VAULT
                .try_with(|context| {
                    context
                        .user
                        .as_ref()
                        .map_or(context.role == VaultRole::Owner, |user| {
                            context.role.permits(
                                &VaultAction::DeleteDocument {
                                    uploaded_by: d.uploaded_by.clone(),
                                },
                                user,
                            )
                        })
                })
                .unwrap_or(false),
            id: d.id,
            title: d.title,
            purge_pending: d.purge_requested_at.is_some(),
            subline: d
                .deleted_at
                .map(|at| format!("Moved to Trash {}", at.format("%Y-%m-%d")))
                .unwrap_or_default(),
        })
        .collect();
    let page = TrashTemplate {
        sidebar: Sidebar::load(&state, vault, Nav::Trash).await?,
        rows,
    };
    Ok(Html(
        page.render()
            .map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

async fn restore_page(
    vault: DefaultVault,
    user: Uploader,
    state: State<AppState>,
    id: ApiPath<Uuid>,
) -> ApiResult<Redirect> {
    let _ = restore(vault, user, state, id).await?;
    Ok(Redirect::to("/trash"))
}

async fn purge_page(
    vault: DefaultVault,
    user: Uploader,
    state: State<AppState>,
    id: ApiPath<Uuid>,
) -> ApiResult<Redirect> {
    purge(vault, user, state, id).await?;
    Ok(Redirect::to("/trash"))
}
