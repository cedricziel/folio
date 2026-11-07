//! Ingestion source API routes

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use folio_core::models::VaultId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use folio_core::error::Error as DomainError;
use folio_core::models::{
    Id, IngestionSourceDef, IngestionSourceInput, SourceConfig, SourceOrigin, SourceStatus,
    REDACTED,
};

use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath};
use crate::openapi::{BadRequest, InternalError, NotFound};
use crate::state::AppState;
use crate::vault::{DefaultVault, CURRENT_VAULT};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_sources).post(create_source))
        .route(
            "/{id}",
            get(get_source).put(update_source).delete(delete_source),
        )
        .route("/{id}/test", post(test_source))
        .route("/{id}/restore", post(restore_source))
        .route("/{id}/outcomes", get(list_source_outcomes))
}

/// A source with its latest run
#[derive(Debug, Serialize, ToSchema)]
pub struct SourceView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Id,
    #[schema(value_type = uuid::Uuid)]
    pub vault_id: VaultId,
    pub name: String,
    pub enabled: bool,
    /// Sources from the server configuration are read-only
    pub origin: SourceOrigin,
    /// Passwords are masked
    pub config: SourceConfig,
    pub status: Option<SourceStatus>,
    /// Configured sources have no persisted lifecycle metadata.
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    #[schema(value_type = Option<String>)]
    pub created_by: Option<folio_core::models::UserId>,
    #[schema(value_type = Option<String>)]
    pub updated_by: Option<folio_core::models::UserId>,
}

pub(crate) async fn view(
    DefaultVault(_vault): DefaultVault,
    state: &AppState,
    def: IngestionSourceDef,
) -> ApiResult<SourceView> {
    let vault_id = match def.vault_id {
        Some(vault) => vault,
        None => state.vaults.default_vault(None).await?,
    };
    let status = state.ingestion.managed.status(def.id).await?;
    Ok(SourceView {
        id: def.id,
        vault_id,
        name: def.name,
        enabled: def.enabled,
        origin: def.origin,
        config: def.config.redacted(),
        status,
        created_at: def.lifecycle.as_ref().map(|m| m.created_at),
        updated_at: def.lifecycle.as_ref().map(|m| m.updated_at),
        deleted_at: def.lifecycle.as_ref().and_then(|m| m.deleted_at),
        created_by: def.lifecycle.as_ref().and_then(|m| m.created_by.clone()),
        updated_by: def.lifecycle.as_ref().and_then(|m| m.updated_by.clone()),
    })
}

/// Configured sources first, then the ones created in the UI
pub(crate) async fn all_sources(
    DefaultVault(vault): DefaultVault,
    state: &AppState,
) -> ApiResult<Vec<IngestionSourceDef>> {
    let instance_vault = if state
        .ingestion
        .configured
        .iter()
        .any(|source| source.vault_id.is_none())
    {
        Some(state.vaults.default_vault(None).await?)
    } else {
        None
    };
    let mut all: Vec<_> = state
        .ingestion
        .configured
        .iter()
        .filter(|source| source.vault_id.or(instance_vault) == Some(vault))
        .cloned()
        .collect();
    all.extend(state.ingestion.managed.list(vault).await?);
    Ok(all)
}

pub(crate) async fn find(
    vault: VaultId,
    state: &AppState,
    id: Id,
) -> ApiResult<IngestionSourceDef> {
    all_sources(DefaultVault(vault), state)
        .await?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| ApiError::not_found("Source not found"))
}

pub(crate) async fn find_retained(
    vault: VaultId,
    state: &AppState,
    id: Id,
) -> ApiResult<IngestionSourceDef> {
    if let Some(def) = all_sources(DefaultVault(vault), state)
        .await?
        .into_iter()
        .find(|d| d.id == id)
    {
        return Ok(def);
    }
    state
        .ingestion
        .managed
        .find_deleted(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Source not found"))
}

#[derive(Default, Deserialize)]
pub(crate) struct SourceListQuery {
    #[serde(default)]
    pub deleted: bool,
}

pub(crate) fn require_managed(def: &IngestionSourceDef) -> ApiResult<()> {
    if def.origin == SourceOrigin::Config {
        return Err(ApiError::bad_request(
            "This source comes from the server configuration and cannot be changed here",
        ));
    }
    Ok(())
}

/// Paperless sources need an instance admin who owns the vault
pub(crate) async fn require_paperless_admin(state: &AppState) -> ApiResult<()> {
    let (user, role) = CURRENT_VAULT
        .try_with(|ctx| (ctx.user.clone(), Some(ctx.role)))
        .unwrap_or((None, None));
    let caller = state.callers.resolve(user.as_ref()).await?;
    folio_core::services::authorize_paperless(&caller, role)?;
    Ok(())
}

/// Applies [`require_paperless_admin`] when any of `configs` is a Paperless source
pub(crate) async fn guard_paperless<'a>(
    state: &AppState,
    configs: impl IntoIterator<Item = &'a SourceConfig>,
) -> ApiResult<()> {
    if configs.into_iter().any(SourceConfig::is_paperless) {
        require_paperless_admin(state).await?;
    }
    Ok(())
}

/// Refuses a second live Paperless source in the vault; `except` keeps its own slot
pub(crate) async fn claim_paperless_slot(
    state: &AppState,
    vault: VaultId,
    config: &SourceConfig,
    except: Option<Id>,
) -> Result<(), DomainError> {
    if !config.is_paperless() {
        return Ok(());
    }
    folio_core::services::ensure_paperless_slot(
        state.ingestion.managed.as_ref(),
        &state.ingestion.configured,
        vault,
        except,
    )
    .await
}

/// Replace a masked secret by the stored one and refuse server-side secret files
///
/// A password or token file would let a UI user make the server read any file
/// and send its content to a host of their choosing, so files are
/// configuration-only. A stored Paperless token is only kept while the URL
/// stays the same, so it is never sent to a host the user just typed in.
pub(crate) fn prepare_input(
    input: &mut IngestionSourceInput,
    existing: Option<&IngestionSourceDef>,
) -> Result<(), DomainError> {
    let previous = existing.map(|d| &d.config);
    match &mut input.config {
        SourceConfig::Email(email) => {
            if email.password_file.is_some() {
                return Err(DomainError::Validation(
                    "Password files can only be set in the server configuration".into(),
                ));
            }
            if email.password.as_deref() == Some(REDACTED) || email.password.is_none() {
                let stored = match previous {
                    Some(SourceConfig::Email(old)) => old.password.clone(),
                    _ => None,
                };
                email.password = Some(stored.ok_or_else(|| {
                    DomainError::Validation("Enter the account password".to_string())
                })?);
            }
        }
        SourceConfig::Paperless(paperless) => {
            if paperless.token_file.is_some() {
                return Err(DomainError::Validation(
                    "token_file can only be set in the server configuration".into(),
                ));
            }
            if paperless
                .token
                .as_deref()
                .is_none_or(|t| t.is_empty() || t == REDACTED)
            {
                paperless.token = Some(match previous {
                    Some(SourceConfig::Paperless(old)) if old.url == paperless.url => old
                        .token
                        .clone()
                        .ok_or_else(|| DomainError::Validation("token is required".to_string()))?,
                    Some(SourceConfig::Paperless(_)) => {
                        return Err(DomainError::Validation(
                            "token: enter the API token again, because the url changed".to_string(),
                        ))
                    }
                    _ => return Err(DomainError::Validation("token is required".to_string())),
                });
            }
        }
        SourceConfig::Folder(_) => {}
    }
    input.validate()
}

/// List ingestion sources
#[utoipa::path(
    get,
    path = "/api/sources",
    operation_id = "listSources",
    tag = "sources",
    params(("deleted" = Option<bool>, Query, description = "List deleted managed sources instead of active sources")),
    responses(
        (status = 200, description = "Configured and managed sources with their latest run", body = Vec<SourceView>),
        InternalError,
    )
)]
pub(crate) async fn list_sources(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Query(query): Query<SourceListQuery>,
) -> ApiResult<Json<Vec<SourceView>>> {
    let mut views = Vec::new();
    let definitions = if query.deleted {
        state.ingestion.managed.list_deleted(vault).await?
    } else {
        all_sources(DefaultVault(vault), &state).await?
    };
    for def in definitions {
        views.push(view(DefaultVault(vault), &state, def).await?);
    }
    Ok(Json(views))
}

/// Get an ingestion source
#[utoipa::path(
    get,
    path = "/api/sources/{id}",
    operation_id = "getSource",
    tag = "sources",
    params(("id" = Uuid, Path, description = "Source ID")),
    responses(
        (status = 200, description = "The source", body = SourceView),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn get_source(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<SourceView>> {
    let def = find(vault, &state, id).await?;
    Ok(Json(view(DefaultVault(vault), &state, def).await?))
}

/// Create an ingestion source
///
/// Passwords are stored unencrypted in the database until Folio has a secret key.
/// Every operation on a Paperless source needs an instance admin who owns the vault.
#[utoipa::path(
    post,
    path = "/api/sources",
    operation_id = "createSource",
    tag = "sources",
    request_body = IngestionSourceInput,
    responses(
        (status = 201, description = "The created source", body = SourceView),
        (status = 409, description = "An active source already uses this name, or the vault already has a Paperless source"),
        BadRequest,
        InternalError,
    )
)]
pub(crate) async fn create_source(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiJson(mut input): ApiJson<IngestionSourceInput>,
) -> ApiResult<(StatusCode, Json<SourceView>)> {
    guard_paperless(&state, [&input.config]).await?;
    prepare_input(&mut input, None)?;
    claim_paperless_slot(&state, vault, &input.config, None).await?;
    let def = state.ingestion.managed.create(vault, input).await?;
    Ok((
        StatusCode::CREATED,
        Json(view(DefaultVault(vault), &state, def).await?),
    ))
}

/// Replace an ingestion source
///
/// A password of `********` keeps the stored one.
#[utoipa::path(
    put,
    path = "/api/sources/{id}",
    operation_id = "updateSource",
    tag = "sources",
    params(("id" = Uuid, Path, description = "Source ID")),
    request_body = IngestionSourceInput,
    responses(
        (status = 200, description = "The updated source", body = SourceView),
        (status = 409, description = "An active source already uses this name, or the vault already has a Paperless source"),
        BadRequest,
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn update_source(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(mut input): ApiJson<IngestionSourceInput>,
) -> ApiResult<Json<SourceView>> {
    let existing = find(vault, &state, id).await?;
    guard_paperless(&state, [&existing.config, &input.config]).await?;
    require_managed(&existing)?;
    prepare_input(&mut input, Some(&existing))?;
    claim_paperless_slot(&state, vault, &input.config, Some(id)).await?;
    let def = state.ingestion.managed.update(vault, id, input).await?;
    Ok(Json(view(DefaultVault(vault), &state, def).await?))
}

/// Move a managed source to trash, retaining its settings and import history
#[utoipa::path(
    delete,
    path = "/api/sources/{id}",
    operation_id = "deleteSource",
    tag = "sources",
    params(("id" = Uuid, Path, description = "Source ID")),
    responses(
        (status = 204, description = "The source is in trash"),
        BadRequest,
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn delete_source(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    let existing = find_retained(vault, &state, id).await?;
    guard_paperless(&state, [&existing.config]).await?;
    require_managed(&existing)?;
    state.ingestion.managed.delete(vault, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Restore a managed source with its previous settings.
#[utoipa::path(
    post,
    path = "/api/sources/{id}/restore",
    operation_id = "restoreSource",
    tag = "sources",
    params(("id" = Uuid, Path, description = "Source ID")),
    responses(
        (status = 200, description = "The restored source", body = SourceView),
        (status = 409, description = "An active source already uses this name, or the vault already has a Paperless source"),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn restore_source(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<SourceView>> {
    let restored = restore(&state, vault, id).await?;
    Ok(Json(view(DefaultVault(vault), &state, restored).await?))
}

/// Restore a retained source, applying the Paperless rules first
pub(crate) async fn restore(
    state: &AppState,
    vault: VaultId,
    id: Id,
) -> ApiResult<IngestionSourceDef> {
    if let Some(retained) = state.ingestion.managed.find_deleted(vault, id).await? {
        guard_paperless(state, [&retained.config]).await?;
        claim_paperless_slot(state, vault, &retained.config, Some(id)).await?;
    }
    Ok(state.ingestion.managed.restore(vault, id).await?)
}

/// Outcome of checking a source
#[derive(Debug, Serialize, ToSchema)]
pub struct SourceTestResult {
    pub ok: bool,
    /// What was found, or why the check failed
    pub message: String,
}

pub(crate) async fn run_test(state: &AppState, def: &IngestionSourceDef) -> SourceTestResult {
    if def.config.is_paperless() {
        return SourceTestResult {
            ok: false,
            message: "unsupported".to_string(),
        };
    }
    let outcome = match state.ingestion.factory.build(def) {
        Ok(source) => source.test().await,
        Err(e) => Err(e),
    };
    match outcome {
        Ok(message) => SourceTestResult { ok: true, message },
        Err(e) => SourceTestResult {
            ok: false,
            message: e.to_string(),
        },
    }
}

/// Check that a source can be reached
///
/// Folders must be readable; mail accounts must accept the login and have the
/// configured mailboxes. Nothing is imported.
#[utoipa::path(
    post,
    path = "/api/sources/{id}/test",
    operation_id = "testSource",
    tag = "sources",
    params(("id" = Uuid, Path, description = "Source ID")),
    responses(
        (status = 200, description = "The check ran; `ok` tells whether it passed", body = SourceTestResult),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn test_source(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<SourceTestResult>> {
    let def = find(vault, &state, id).await?;
    guard_paperless(&state, [&def.config]).await?;
    Ok(Json(run_test(&state, &def).await))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutcomeQuery {
    pub before: Option<i64>,
    #[serde(default = "default_outcome_limit")]
    pub limit: u32,
}

fn default_outcome_limit() -> u32 {
    50
}

pub(crate) async fn outcome_page(
    state: &AppState,
    vault: VaultId,
    source_id: Id,
    query: OutcomeQuery,
) -> ApiResult<folio_core::models::IngestOutcomePage> {
    let mut page = state
        .ingestion
        .outcomes
        .page(vault, source_id, query.before, query.limit)
        .await?;
    for outcome in &mut page.outcomes {
        if let Some(document_id) = outcome.document_id {
            if state
                .document_repo
                .find_by_id(vault, document_id)
                .await?
                .is_none()
            {
                outcome.document_id = None;
            }
        }
    }
    Ok(page)
}

/// Read committed item outcomes for an ingestion source.
#[utoipa::path(
    get,
    path = "/api/sources/{id}/outcomes",
    operation_id = "listSourceOutcomes",
    tag = "sources",
    params(
        ("id" = Uuid, Path, description = "Source ID"),
        ("before" = Option<i64>, Query, minimum = 1, description = "Return outcomes older than this positive outcome ID"),
        ("limit" = Option<u32>, Query, minimum = 1, maximum = 100, description = "Page size, from 1 to 100; defaults to 50"),
    ),
    responses(
        (status = 200, description = "Recent source outcomes", body = folio_core::models::IngestOutcomePage),
        BadRequest,
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn list_source_outcomes(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    crate::extract::ApiQuery(query): crate::extract::ApiQuery<OutcomeQuery>,
) -> ApiResult<Json<folio_core::models::IngestOutcomePage>> {
    find(vault, &state, id).await?;
    Ok(Json(outcome_page(&state, vault, id, query).await?))
}
