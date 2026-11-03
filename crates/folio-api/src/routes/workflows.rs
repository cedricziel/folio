//! Workflow API routes

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use folio_core::models::VaultId;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use folio_core::models::{
    redact_url, WebhookPayload, Workflow, WorkflowAction, WorkflowInput, WorkflowRun,
};
use folio_core::ports::WorkflowRepository;
use folio_infra::{workflow::WebhookRequest, WebhookClient};

use crate::error::{ApiError, ApiResult};
use crate::extract::{ApiJson, ApiPath, ApiQuery};
use crate::openapi::{BadRequest, InternalError, NotFound};
use crate::state::AppState;
use crate::vault::DefaultVault;

/// Placeholder returned in place of stored webhook secrets
///
/// Sending it back unchanged on update keeps the stored secret.
pub const SECRET_MASK: &str = "********";

const DEFAULT_RUN_LIMIT: u32 = 50;
const MAX_RUN_LIMIT: u32 = 200;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_workflows).post(create_workflow))
        .route(
            "/{id}",
            get(get_workflow)
                .put(update_workflow)
                .delete(delete_workflow),
        )
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
pub(crate) fn restore_secrets(
    input: &mut WorkflowInput,
    existing: &Workflow,
) -> Result<(), ApiError> {
    for (index, action) in input.actions.iter_mut().enumerate() {
        let WorkflowAction::Webhook {
            url,
            secret,
            headers,
            ..
        } = action
        else {
            continue;
        };
        let stored = match existing.actions.get(index) {
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
            let kept = stored.and_then(|(s, _)| s.clone());
            *secret = Some(kept.ok_or_else(|| cannot_keep("Webhook secret"))?);
        }
        for (name, value) in headers
            .iter_mut()
            .filter(|(_, v)| v.as_str() == SECRET_MASK)
        {
            let kept = stored.and_then(|(_, h)| h.get(name).cloned());
            *value = kept.ok_or_else(|| cannot_keep(&format!("Header {name}")))?;
        }
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
    ApiJson(mut input): ApiJson<WorkflowInput>,
) -> ApiResult<Json<Workflow>> {
    let existing = find(vault, &*state.workflow_repo, id).await?;
    restore_secrets(&mut input, &existing)?;
    let workflow = state.workflow_repo.update(vault, id, input).await?;
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
    let result = send_test(&state.webhook_client, &workflow, &state.public_url).await?;
    Ok(Json(result))
}

pub(crate) async fn send_test(
    client: &WebhookClient,
    workflow: &Workflow,
    public_url: &str,
) -> ApiResult<TestResult> {
    let payload = WebhookPayload::sample(workflow.trigger, public_url);
    let body = serde_json::to_string(&payload)
        .map_err(|e| ApiError::internal(format!("Failed to encode payload: {e}")))?;

    let mut deliveries = Vec::new();
    let run_id = Uuid::new_v4();
    for (action_index, action) in workflow.actions.iter().enumerate() {
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
                event: workflow.trigger,
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
    Ok(Json(state.workflow_repo.list_runs(vault, id, limit).await?))
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
