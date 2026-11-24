//! Workflow API routes

use std::collections::HashSet;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, patch, post, put},
    Json, Router,
};
use folio_core::models::VaultId;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use folio_core::models::{
    redact_url, WebhookPayload, Workflow, WorkflowAction, WorkflowDocumentConditions,
    WorkflowInput, WorkflowRun, WorkflowTrigger, WorkflowTriggerDefinition,
    WorkflowTriggerRegistry,
};
use folio_core::ports::WorkflowRepository;
use folio_infra::{workflow::WebhookRequest, WebhookClient};

use crate::error::{ApiError, ApiResult, ErrorResponse};
use crate::extract::{ApiJson, ApiPath, ApiQuery};
use crate::openapi::{BadRequest, InternalError, NotFound};
use crate::state::AppState;
use crate::vault::DefaultVault;

/// Placeholder returned in place of stored webhook secrets
///
/// Sending it back unchanged on update keeps the stored secret.
pub const SECRET_MASK: &str = "********";

const ACTION_SOURCES: &str = "x-folio-workflow-action-sources";

const DEFAULT_RUN_LIMIT: u32 = 50;
const MAX_RUN_LIMIT: u32 = 200;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_workflows).post(create_workflow))
        .route("/triggers", get(list_triggers))
        .route(
            "/{id}",
            get(get_workflow)
                .put(update_workflow)
                .delete(delete_workflow),
        )
        .route("/{id}/trigger", patch(update_workflow_trigger))
        .route("/{id}/definition", put(update_workflow_definition))
        .route("/{id}/test", post(test_workflow))
        .route("/{id}/restore", post(restore_workflow))
        .route("/{id}/runs", get(list_runs))
}

/// Hide webhook secrets and header values before a workflow leaves the server
///
/// Header values often carry credentials (`Authorization`), so they get the same treatment.
pub(crate) fn redact(mut workflow: Workflow) -> Workflow {
    for action in &mut workflow.actions {
        if let WorkflowAction::Webhook {
            secret, headers, ..
        } = action
        {
            if secret.as_deref().is_some_and(|s| !s.is_empty()) {
                *secret = Some(SECRET_MASK.to_string());
            }
            for value in headers.values_mut().filter(|v| !v.is_empty()) {
                *value = SECRET_MASK.to_string();
            }
        }
    }
    workflow
}

/// Swap [`SECRET_MASK`] placeholders for the values stored on the workflow being replaced
///
/// Values are only restored when the webhook keeps its URL, so a stored secret
/// can never be redirected to a new destination.
fn restore_secrets(
    input: &mut WorkflowInput,
    existing: &Workflow,
    sources: Option<&str>,
) -> Result<(), ApiError> {
    let mut indices = sources.map(|raw| raw.split(','));
    let mut used = HashSet::new();
    let invalid = || {
        ApiError::bad_request(
            "Action sources must name one distinct saved action index or '-' for each action",
        )
    };
    for (index, action) in input.actions.iter_mut().enumerate() {
        let saved = match indices.as_mut() {
            None => existing.actions.get(index),
            Some(indices) => match indices.next().ok_or_else(invalid)? {
                "-" => None,
                raw => {
                    let source = raw.parse::<usize>().map_err(|_| invalid())?;
                    let saved = existing.actions.get(source).ok_or_else(invalid)?;
                    if !used.insert(source) {
                        return Err(invalid());
                    }
                    Some(saved)
                }
            },
        };
        restore_webhook_secrets(action, saved)?;
    }
    if indices.is_some_and(|mut values| values.next().is_some()) {
        return Err(invalid());
    }
    Ok(())
}

fn edit_header<'a>(headers: &'a HeaderMap, name: &str) -> ApiResult<Option<&'a str>> {
    let mut values = headers.get_all(name).into_iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(ApiError::bad_request(format!(
            "Provide only one {name} header"
        )));
    }
    value
        .to_str()
        .map(Some)
        .map_err(|_| ApiError::bad_request(format!("Invalid {name} header")))
}

pub(super) fn restore_webhook_secrets(
    action: &mut WorkflowAction,
    saved: Option<&WorkflowAction>,
) -> Result<(), ApiError> {
    let WorkflowAction::Webhook {
        url,
        secret,
        headers,
        ..
    } = action
    else {
        return Ok(());
    };
    let stored = match saved {
        Some(WorkflowAction::Webhook {
            url: stored_url,
            secret,
            headers,
            ..
        }) if stored_url == url => Some((secret, headers)),
        _ => None,
    };
    let cannot_keep = |what: &str| {
        ApiError::bad_request(format!(
            "{what} for {} cannot be kept; provide it again",
            redact_url(url)
        ))
    };
    if secret.as_deref() == Some(SECRET_MASK) {
        let kept = stored.and_then(|(value, _)| value.clone());
        *secret = Some(kept.ok_or_else(|| cannot_keep("Webhook secret"))?);
    }
    for (name, value) in headers
        .iter_mut()
        .filter(|(_, value)| value.as_str() == SECRET_MASK)
    {
        let kept = stored.and_then(|(_, saved)| saved.get(name).cloned());
        *value = kept.ok_or_else(|| cannot_keep(&format!("Header {name}")))?;
    }
    Ok(())
}

/// The mask is only meaningful when updating; on create it would be stored literally
fn reject_masks(input: &WorkflowInput) -> Result<(), ApiError> {
    let masked = input.actions.iter().any(|action| match action {
        WorkflowAction::Webhook {
            secret, headers, ..
        } => secret.as_deref() == Some(SECRET_MASK) || headers.values().any(|v| v == SECRET_MASK),
        _ => false,
    });
    if masked {
        return Err(ApiError::bad_request(format!(
            "'{SECRET_MASK}' only keeps stored values on update; provide the real value"
        )));
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct WorkflowListQuery {
    /// List retained workflows in trash instead of active workflows.
    #[serde(default)]
    pub deleted: bool,
}

// The original native picker can only encode the trigger IDs in its generated request type.
const LEGACY_WORKFLOW_TRIGGER_IDS: &[&str] = &[
    "document.uploaded",
    "document.processed",
    "document.updated",
    "document.title_changed",
    "document.tags_changed",
    "document.correspondent_changed",
    "document.document_type_changed",
    "document_note.created",
    "document_note.updated",
    "document_note.deleted",
    "document_comment.created",
    "document_comment.updated",
    "document_comment.deleted",
    "metadata_field.created",
    "metadata_field.updated",
    "metadata_field.deleted",
    "metadata_field.restored",
    "metadata_option.created",
    "metadata_option.updated",
    "metadata_option.deleted",
    "metadata_option.restored",
    "document.metadata.changed",
    "document.moved_out",
    "document.moved_in",
    "document.trashed",
    "document.restored",
    "document.purge_requested",
    "document.purged",
    "document.deleted",
    "ingest.received",
    "ingest.imported",
    "ingest.skipped",
    "ingest.duplicate",
    "ingest.failed",
    "source.failed",
    "source.recovered",
    "document.processing.ocr.started",
    "document.processing.ocr.completed",
    "document.processing.ocr.failed",
    "document.processing.ocr.skipped",
    "document.processing.ocr.retry_scheduled",
    "document.processing.embedding.started",
    "document.processing.embedding.completed",
    "document.processing.embedding.failed",
    "document.processing.embedding.skipped",
    "document.processing.embedding.retry_scheduled",
    "document.processing.thumbnail.started",
    "document.processing.thumbnail.completed",
    "document.processing.thumbnail.failed",
    "document.processing.thumbnail.skipped",
    "document.processing.thumbnail.retry_scheduled",
    "workflow.scheduled",
];

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct WorkflowTriggerCatalogQuery {
    /// Include newer trigger IDs. Omit this parameter for the original picker catalog.
    pub include_extended: Option<bool>,
}

/// List selectable workflow triggers and their occurrence fields
#[utoipa::path(
    get,
    path = "/api/workflows/triggers",
    operation_id = "listWorkflowTriggers",
    tag = "workflows",
    params(WorkflowTriggerCatalogQuery),
    responses(
        (status = 200, description = "Selectable workflow triggers", body = Vec<WorkflowTriggerDefinition>,
            headers(("Folio-Workflow-Document-Conditions" = String, description = "The server accepts guarded document predicate edits when true"))),
        BadRequest,
    )
)]
pub(crate) async fn list_triggers(
    DefaultVault(_vault): DefaultVault,
    ApiQuery(query): ApiQuery<WorkflowTriggerCatalogQuery>,
) -> impl IntoResponse {
    let extended = query.include_extended.unwrap_or_default();
    let definitions: Vec<_> = WorkflowTriggerRegistry::built_in()
        .definitions()
        .iter()
        .filter(|definition| {
            extended || LEGACY_WORKFLOW_TRIGGER_IDS.contains(&definition.event_type.as_str())
        })
        .collect();
    (
        [("folio-workflow-document-conditions", "true")],
        Json(definitions),
    )
}

/// List all workflows
#[utoipa::path(
    get,
    path = "/api/workflows",
    operation_id = "listWorkflows",
    tag = "workflows",
    params(WorkflowListQuery),
    responses(
        (status = 200, description = "All workflows; webhook secrets are masked", body = Vec<Workflow>),
        InternalError,
    )
)]
pub(crate) async fn list_workflows(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<WorkflowListQuery>,
) -> ApiResult<Json<Vec<Workflow>>> {
    let workflows = if query.deleted {
        state.workflow_repo.list_deleted(vault).await?
    } else {
        state.workflow_repo.list(vault).await?
    };
    Ok(Json(workflows.into_iter().map(redact).collect()))
}

/// Get a workflow
#[utoipa::path(
    get,
    path = "/api/workflows/{id}",
    operation_id = "getWorkflow",
    tag = "workflows",
    params(("id" = Uuid, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "The workflow; webhook secrets are masked", body = Workflow),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn get_workflow(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<Workflow>> {
    let workflow = find(vault, &*state.workflow_repo, id).await?;
    Ok(Json(redact(workflow)))
}

/// Create a workflow
#[utoipa::path(
    post,
    path = "/api/workflows",
    operation_id = "createWorkflow",
    tag = "workflows",
    request_body = WorkflowInput,
    responses(
        (status = 201, description = "The created workflow", body = Workflow),
        BadRequest,
        InternalError,
    )
)]
pub(crate) async fn create_workflow(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiJson(input): ApiJson<WorkflowInput>,
) -> ApiResult<(StatusCode, Json<Workflow>)> {
    reject_masks(&input)?;
    let workflow = state.workflow_repo.create(vault, input).await?;
    Ok((StatusCode::CREATED, Json(redact(workflow))))
}

/// Replace a workflow
///
/// All fields are replaced. A webhook `secret` of `********` keeps the stored one.
#[utoipa::path(
    put,
    path = "/api/workflows/{id}",
    operation_id = "updateWorkflow",
    tag = "workflows",
    params(("id" = Uuid, Path, description = "Workflow ID")),
    request_body = WorkflowInput,
    responses(
        (status = 200, description = "The updated workflow", body = Workflow),
        BadRequest,
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn update_workflow(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<WorkflowInput>,
) -> ApiResult<Json<Workflow>> {
    replace_workflow(&state, vault, id, input, None, None).await
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct WorkflowRevisionQuery {
    /// Positive revision returned by the workflow read used to start editing.
    expected_revision: i64,
}

/// Replace a workflow only if it still matches the editor's last read.
#[utoipa::path(
    put,
    path = "/api/workflows/{id}/definition",
    operation_id = "updateWorkflowDefinition",
    tag = "workflows",
    params(
        ("id" = Uuid, Path, description = "Workflow ID"),
        WorkflowRevisionQuery,
        ("X-Folio-Workflow-Action-Sources" = Option<Vec<String>>, Header,
            description = "One distinct saved action index per action in the new order, or '-' for a new action."),
    ),
    request_body = WorkflowInput,
    responses(
        (status = 200, description = "The updated workflow; webhook secrets are masked", body = Workflow),
        (status = 409, description = "The workflow changed after the revision was read", body = ErrorResponse),
        BadRequest,
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn update_workflow_definition(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<WorkflowRevisionQuery>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<WorkflowInput>,
) -> ApiResult<Json<Workflow>> {
    if query.expected_revision < 1 {
        return Err(ApiError::bad_request("Expected revision must be positive"));
    }
    let sources = edit_header(&headers, ACTION_SOURCES)?;
    replace_workflow(
        &state,
        vault,
        id,
        input,
        Some(query.expected_revision),
        sources,
    )
    .await
}

async fn replace_workflow(
    state: &AppState,
    vault: VaultId,
    id: Uuid,
    mut input: WorkflowInput,
    expected_revision: Option<i64>,
    action_sources: Option<&str>,
) -> ApiResult<Json<Workflow>> {
    let existing = find(vault, &*state.workflow_repo, id).await?;
    existing.check_revision(expected_revision)?;
    restore_secrets(&mut input, &existing, action_sources)?;
    let workflow = state
        .workflow_repo
        .replace(vault, id, input, expected_revision)
        .await?;
    Ok(Json(redact(workflow)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct WorkflowTriggerUpdate {
    /// Revision returned by the workflow read used to start editing.
    #[schema(minimum = 1)]
    expected_revision: i64,
    trigger: WorkflowTrigger,
    /// Omit to retain the document predicate. Use null to clear it.
    #[serde(default, deserialize_with = "deserialize_document_predicate")]
    #[schema(value_type = Option<WorkflowDocumentConditions>, required = false)]
    document_change: Option<Option<WorkflowDocumentConditions>>,
}

fn deserialize_document_predicate<'de, D>(
    deserializer: D,
) -> Result<Option<Option<WorkflowDocumentConditions>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<WorkflowDocumentConditions>::deserialize(deserializer).map(Some)
}

/// Change a workflow's trigger and optional document predicate.
#[utoipa::path(
    patch,
    path = "/api/workflows/{id}/trigger",
    operation_id = "updateWorkflowTrigger",
    tag = "workflows",
    params(("id" = Uuid, Path, description = "Workflow ID")),
    request_body = WorkflowTriggerUpdate,
    responses(
        (status = 200, description = "The updated workflow; webhook secrets are masked", body = Workflow),
        (status = 409, description = "The workflow changed after the revision was read", body = ErrorResponse),
        BadRequest,
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn update_workflow_trigger(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(input): ApiJson<WorkflowTriggerUpdate>,
) -> ApiResult<Json<Workflow>> {
    if input.expected_revision < 1 {
        return Err(ApiError::bad_request("Expected revision must be positive"));
    }
    let existing = find(vault, &*state.workflow_repo, id).await?;
    let expected_revision = Some(input.expected_revision);
    existing.check_revision(expected_revision)?;
    let triggers = std::iter::once(input.trigger)
        .chain(
            existing
                .triggers
                .into_iter()
                .skip(1)
                .filter(|trigger| *trigger != input.trigger),
        )
        .collect();
    let mut conditions = existing.conditions;
    if let Some(document_change) = input.document_change {
        let event = conditions.event.get_or_insert_with(Default::default);
        event.document_change = document_change;
        if *event == Default::default() {
            conditions.event = None;
        }
    }
    let replacement = WorkflowInput {
        name: existing.name,
        enabled: existing.enabled,
        trigger: input.trigger,
        triggers,
        cadence: existing.cadence,
        conditions,
        actions: existing.actions,
    };
    let workflow = state
        .workflow_repo
        .replace(vault, id, replacement, expected_revision)
        .await?;
    Ok(Json(redact(workflow)))
}

/// Move a workflow to trash, preserving history and stopping pending runs
#[utoipa::path(
    delete,
    path = "/api/workflows/{id}",
    operation_id = "deleteWorkflow",
    tag = "workflows",
    params(("id" = Uuid, Path, description = "Workflow ID")),
    responses(
        (status = 204, description = "The workflow is in trash; pending runs are stopped"),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn delete_workflow(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<StatusCode> {
    state.workflow_repo.delete(vault, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Restore the configuration without restarting retained execution history.
#[utoipa::path(
    post,
    path = "/api/workflows/{id}/restore",
    operation_id = "restoreWorkflow",
    tag = "workflows",
    params(("id" = Uuid, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "The restored workflow; secrets are masked", body = Workflow),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn restore_workflow(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<Workflow>> {
    let workflow = state.workflow_repo.restore(vault, id).await?;
    Ok(Json(redact(workflow)))
}

/// Outcome of sending the sample payload to one webhook
#[derive(Debug, Serialize, ToSchema)]
pub struct TestDelivery {
    pub url: String,
    /// The receiver answered with a 2xx status
    pub success: bool,
    /// HTTP status of the response, when one arrived
    pub status: Option<u16>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Result of "send test"
#[derive(Debug, Serialize, ToSchema)]
pub struct TestResult {
    /// What was sent
    pub payload: WebhookPayload,
    /// One entry per webhook action, in order
    pub deliveries: Vec<TestDelivery>,
}

/// Send a sample payload to the workflow's webhooks
///
/// Delivers once, signed like a real delivery, without retries. Nothing is recorded in the run history.
#[utoipa::path(
    post,
    path = "/api/workflows/{id}/test",
    operation_id = "testWorkflow",
    tag = "workflows",
    params(("id" = Uuid, Path, description = "Workflow ID")),
    responses(
        (status = 200, description = "Delivery results; a failing receiver is reported here, not as an error", body = TestResult),
        BadRequest,
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn test_workflow(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
) -> ApiResult<Json<TestResult>> {
    let workflow = find(vault, &*state.workflow_repo, id).await?;
    let result = send_test(
        &state.webhook_client,
        workflow.trigger,
        &workflow.actions,
        &state.public_url,
    )
    .await?;
    Ok(Json(result))
}

pub(crate) async fn send_test(
    client: &WebhookClient,
    trigger: WorkflowTrigger,
    actions: &[WorkflowAction],
    public_url: &str,
) -> ApiResult<TestResult> {
    let payload = WebhookPayload::sample(trigger, public_url);
    let body = serde_json::to_string(&payload)
        .map_err(|e| ApiError::internal(format!("Failed to encode payload: {e}")))?;

    let mut deliveries = Vec::new();
    let run_id = Uuid::new_v4();
    for (action_index, action) in actions.iter().enumerate() {
        let WorkflowAction::Webhook {
            url,
            headers,
            secret,
            ..
        } = action
        else {
            continue;
        };
        let result = client
            .send(WebhookRequest {
                url,
                headers,
                secret: secret.as_deref(),
                event: trigger,
                run_id,
                action_index,
                body: &body,
            })
            .await;
        deliveries.push(TestDelivery {
            url: redact_url(url),
            success: result.is_success(),
            status: result.status,
            error: result.error,
            duration_ms: result.duration_ms,
        });
    }

    if deliveries.is_empty() {
        return Err(ApiError::bad_request("The workflow has no webhook to test"));
    }
    Ok(TestResult {
        payload,
        deliveries,
    })
}

/// Query parameters for listing runs
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRunsQuery {
    /// Maximum number of runs, newest first
    #[param(default = 50, maximum = 200)]
    limit: Option<u32>,
}

/// List a workflow's recent runs, newest first
#[utoipa::path(
    get,
    path = "/api/workflows/{id}/runs",
    operation_id = "listWorkflowRuns",
    tag = "workflows",
    params(("id" = Uuid, Path, description = "Workflow ID"), ListRunsQuery),
    responses(
        (status = 200, description = "Recent runs", body = Vec<WorkflowRun>),
        NotFound,
        InternalError,
    )
)]
pub(crate) async fn list_runs(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiQuery(query): ApiQuery<ListRunsQuery>,
) -> ApiResult<Json<Vec<WorkflowRun>>> {
    find_retained(vault, &*state.workflow_repo, id).await?;
    let limit = query.limit.unwrap_or(DEFAULT_RUN_LIMIT).min(MAX_RUN_LIMIT);
    let mut runs = state.workflow_repo.list_runs(vault, id, limit).await?;
    for run in &mut runs {
        run.last_error = history_error(run);
    }
    Ok(Json(runs))
}

pub(super) fn history_error(run: &WorkflowRun) -> Option<String> {
    let message = run.last_error.as_ref()?;
    if let Some(status) = run.response_status {
        let http_error = format!("HTTP {status}");
        // Older runs may have a receiver-provided response body after the HTTP status.
        if message.starts_with(&http_error) {
            return Some(http_error);
        }
    }

    Some(message.clone())
}

pub(crate) async fn find(
    vault: VaultId,
    repo: &dyn WorkflowRepository,
    id: Uuid,
) -> ApiResult<Workflow> {
    repo.find_by_id(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Workflow not found"))
}

pub(crate) async fn find_retained(
    vault: VaultId,
    repo: &dyn WorkflowRepository,
    id: Uuid,
) -> ApiResult<Workflow> {
    if let Some(workflow) = repo.find_by_id(vault, id).await? {
        return Ok(workflow);
    }
    repo.find_deleted(vault, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Workflow not found"))
}
