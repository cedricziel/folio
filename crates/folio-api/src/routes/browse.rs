use axum::{
    extract::{RawQuery, State},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use folio_core::models::{
    Access, DocumentPageQuery, DocumentSort, SavedViewQuery, VaultAction, VaultId,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::documents::{present, DocumentView, ListDocumentsQuery};
use crate::{
    error::{ApiError, ApiResult},
    extract::{ApiJson, ApiPath},
    state::AppState,
    vault::{DefaultVault, Uploader},
};

#[derive(IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code)]
pub struct BrowseQuery {
    /// Restrict results to the Inbox, including active or failed processing.
    inbox_only: Option<bool>,
    /// Literal words to match in title or content. Every word must match; at most 32 words.
    q: Option<String>,
    /// Maximum documents per page, from 1 to 100. Defaults to 50.
    limit: Option<u32>,
    /// Stable order: newest, oldest, or title. Defaults to newest.
    sort: Option<String>,
    /// Opaque continuation from the same vault, filters, and sort order.
    cursor: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct BrowseResponse {
    #[schema(required = false)]
    supports_document_review_filters: bool,
    vault_id: Uuid,
    documents: Vec<DocumentView>,
    next_cursor: Option<String>,
}

pub(super) fn query(raw: Option<&str>) -> ApiResult<DocumentPageQuery> {
    let mut query = DocumentPageQuery {
        filter: ListDocumentsQuery::filter(raw)?.unwrap_or_default(),
        ..Default::default()
    };
    for (key, value) in url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "inbox_only" => {
                query.filter.inbox_only = value
                    .parse()
                    .map_err(|_| ApiError::bad_request("Invalid Inbox scope"))?;
            }
            "q" => {
                let terms = value.split_whitespace().take(33).collect::<Vec<_>>();
                if terms.len() > 32 {
                    return Err(ApiError::bad_request("Search supports at most 32 words"));
                }
                let text = terms.join(" ");
                query.filter.search_query = (!text.is_empty()).then_some(text);
            }
            "limit" => {
                query.limit = value
                    .parse()
                    .map_err(|_| ApiError::bad_request("Invalid limit"))?
            }
            "sort" => {
                query.sort = match value.as_ref() {
                    "newest" => DocumentSort::Newest,
                    "oldest" => DocumentSort::Oldest,
                    "title" => DocumentSort::Title,
                    _ => return Err(ApiError::bad_request("Invalid sort")),
                }
            }
            "cursor" => {
                query.cursor = Some(decode_cursor(&value)?);
            }
            _ => {}
        }
    }
    Ok(query)
}

fn decode_cursor(value: &str) -> ApiResult<folio_core::models::DocumentCursor> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value.as_bytes())
        .map_err(|_| ApiError::bad_request("Invalid cursor"))?;
    serde_json::from_slice(&bytes).map_err(|_| ApiError::bad_request("Invalid cursor"))
}

async fn browse(
    state: AppState,
    default: VaultId,
    vault: VaultId,
    uploader: Uploader,
    query: ApiResult<DocumentPageQuery>,
) -> ApiResult<Json<BrowseResponse>> {
    let visible = match uploader.0 {
        Some(user) => matches!(
            state.access.can(&user, vault, VaultAction::Read).await?,
            Access::Allowed(_)
        ),
        None => vault == default,
    };
    if !visible {
        return Err(ApiError::not_found("Vault not found"));
    }
    let query = query?;
    if let Some(exact) = &query.filter.metadata_exact {
        super::saved_views::validate_metadata_filter(&state, vault, exact).await?;
    }
    let page = state.document_repo.page(vault, query).await?;
    let next_cursor = page
        .next_cursor
        .map(|cursor| serde_json::to_vec(&cursor).map(|bytes| URL_SAFE_NO_PAD.encode(bytes)))
        .transpose()
        .map_err(|_| ApiError::internal("Could not encode cursor"))?;
    Ok(Json(BrowseResponse {
        supports_document_review_filters: true,
        vault_id: vault.0,
        documents: present(vault, &state, page.documents).await?,
        next_cursor,
    }))
}

#[utoipa::path(get, path = "/api/documents/browse", operation_id = "browseDocuments", tag = "documents",
    params(BrowseQuery, ListDocumentsQuery),
    responses((status = 200, description = "One page from the default vault", body = BrowseResponse),
        (status = 401, description = "Sign-in required"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::InternalError))]
pub(crate) async fn browse_default(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<BrowseResponse>> {
    browse(state, vault, vault, uploader, query(raw.as_deref())).await
}

#[utoipa::path(get, path = "/api/vaults/{vault_id}/documents", operation_id = "browseVaultDocuments", tag = "documents",
    params(("vault_id" = Uuid, Path, description = "Vault to browse"), BrowseQuery, ListDocumentsQuery),
    responses((status = 200, description = "One page from an accessible vault", body = BrowseResponse),
        (status = 401, description = "Sign-in required"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::InternalError))]
pub(crate) async fn browse_vault(
    ApiPath(id): ApiPath<Uuid>,
    DefaultVault(default): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    RawQuery(raw): RawQuery,
) -> ApiResult<Json<BrowseResponse>> {
    browse(state, default, VaultId(id), uploader, query(raw.as_deref())).await
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowseFilteredInput {
    query: SavedViewQuery,
    limit: Option<u32>,
    cursor: Option<String>,
}

#[utoipa::path(post, path = "/api/documents/browse", operation_id = "browseFilteredDocuments", tag = "documents",
    request_body = BrowseFilteredInput,
    responses((status = 200, description = "One page matching the versioned query", body = BrowseResponse),
        (status = 401, description = "Sign-in required"), crate::openapi::BadRequest, crate::openapi::NotFound, crate::openapi::InternalError))]
pub(crate) async fn browse_filtered(
    DefaultVault(vault): DefaultVault,
    uploader: Uploader,
    State(state): State<AppState>,
    ApiJson(input): ApiJson<BrowseFilteredInput>,
) -> ApiResult<Json<BrowseResponse>> {
    let cursor = input.cursor.as_deref().map(decode_cursor).transpose()?;
    let query = input
        .query
        .page(vault, input.limit.unwrap_or(50), cursor)
        .map_err(Into::into);
    browse(state, vault, vault, uploader, query).await
}
