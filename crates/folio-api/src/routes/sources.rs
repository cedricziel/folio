//! Ingestion source API routes

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_sources).post(create_source))
        .route(
            "/{id}",
            get(get_source).put(update_source).delete(delete_source),
        )
        .route("/{id}/test", post(test_source))
}

/// A source with its latest run
#[derive(Debug, Serialize, ToSchema)]
pub struct SourceView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Id,
    pub name: String,
    pub enabled: bool,
    /// Sources from the server configuration are read-only
    pub origin: SourceOrigin,
    /// Passwords are masked
    pub config: SourceConfig,
    pub status: Option<SourceStatus>,
}

pub(crate) async fn view(state: &AppState, def: IngestionSourceDef) -> ApiResult<SourceView> {
    let status = state.ingestion.managed.status(def.id).await?;
    Ok(SourceView {
        id: def.id,
        name: def.name,
        enabled: def.enabled,
        origin: def.origin,
        config: def.config.redacted(),
        status,
    })
}

/// Configured sources first, then the ones created in the UI
pub(crate) async fn all_sources(state: &AppState) -> ApiResult<Vec<IngestionSourceDef>> {
    let mut all: Vec<_> = state.ingestion.configured.iter().cloned().collect();
    all.extend(state.ingestion.managed.list().await?);
    Ok(all)
}

pub(crate) async fn find(state: &AppState, id: Id) -> ApiResult<IngestionSourceDef> {
    all_sources(state)
        .await?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| ApiError::not_found("Source not found"))
}

pub(crate) fn require_managed(def: &IngestionSourceDef) -> ApiResult<()> {
    if def.origin == SourceOrigin::Config {
        return Err(ApiError::bad_request(
            "This source comes from the server configuration and cannot be changed here",
        ));
    }
    Ok(())
}

/// Replace a masked password by the stored one and refuse server-side password files
///
/// A password file would let a UI user make the server read any file and send
/// its content to a host of their choosing, so files are configuration-only.
pub(crate) fn prepare_input(
    input: &mut IngestionSourceInput,
    existing: Option<&IngestionSourceDef>,
) -> Result<(), DomainError> {
    if let SourceConfig::Email(email) = &mut input.config {
        if email.password_file.is_some() {
            return Err(DomainError::Validation(
                "Password files can only be set in the server configuration".into(),
            ));
        }
        if email.password.as_deref() == Some(REDACTED) || email.password.is_none() {
            let stored = match existing.map(|d| &d.config) {
                Some(SourceConfig::Email(old)) => old.password.clone(),
                _ => None,
            };
            email.password = Some(stored.ok_or_else(|| {
                DomainError::Validation("Enter the account password".to_string())
            })?);
        }
    }
    input.validate()
}

/// List ingestion sources
#[utoipa::path(
    get,
    path = "/api/sources",
    operation_id = "listSources",
    tag = "sources",
    responses(
        (status = 200, description = "Configured and managed sources with their latest run", body = Vec<SourceView>),
        InternalError,
    )
)]
pub(crate) async fn list_sources(
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<SourceView>>> {
    let mut views = Vec::new();
    for def in all_sources(&state).await? {
        views.push(view(&state, def).await?);
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
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<SourceView>> {
    let def = find(&state, id).await?;
    Ok(Json(view(&state, def).await?))
}

/// Create an ingestion source
///
/// Passwords are stored unencrypted in the database until Folio has a secret key.
#[utoipa::path(
    post,
    path = "/api/sources",
    operation_id = "createSource",
    tag = "sources",
    request_body = IngestionSourceInput,
    responses(
        (status = 201, description = "The created source", body = SourceView),
        BadRequest,
        InternalError,
    )
)]
pub(crate) async fn create_source(
    State(state): State<AppState>,
    ApiJson(mut input): ApiJson<IngestionSourceInput>,
) -> ApiResult<(StatusCode, Json<SourceView>)> {
    prepare_input(&mut input, None)?;
    let def = state.ingestion.managed.create(input).await?;
    Ok((StatusCode::CREATED, Json(view(&state, def).await?)))
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
        BadRequest,
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn update_source(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(mut input): ApiJson<IngestionSourceInput>,
) -> ApiResult<Json<SourceView>> {
    let existing = find(&state, id).await?;
    require_managed(&existing)?;
    prepare_input(&mut input, Some(&existing))?;
    let def = state.ingestion.managed.update(id, input).await?;
    Ok(Json(view(&state, def).await?))
}

/// Delete an ingestion source
#[utoipa::path(
    delete,
    path = "/api/sources/{id}",
    operation_id = "deleteSource",
    tag = "sources",
    params(("id" = Uuid, Path, description = "Source ID")),
    responses(
        (status = 204, description = "The source was deleted"),
        BadRequest,
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn delete_source(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    let existing = find(&state, id).await?;
    require_managed(&existing)?;
    state.ingestion.managed.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Outcome of checking a source
#[derive(Debug, Serialize, ToSchema)]
pub struct SourceTestResult {
    pub ok: bool,
    /// What was found, or why the check failed
    pub message: String,
}

pub(crate) async fn run_test(state: &AppState, def: &IngestionSourceDef) -> SourceTestResult {
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
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<SourceTestResult>> {
    let def = find(&state, id).await?;
    Ok(Json(run_test(&state, &def).await))
}
