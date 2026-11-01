//! Server-rendered workflow pages: list, create/edit form, test, and run history

use std::collections::BTreeMap;

use askama::Template;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use folio_core::models::{
    Id, RunStatus, WebhookMethod, Workflow, WorkflowAction, WorkflowConditions, WorkflowInput,
    WorkflowRun, WorkflowTrigger,
};

use super::workflows::{find, send_test, SECRET_MASK};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const RUN_HISTORY: u32 = 20;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/workflows", get(list_page).post(create))
        .route("/workflows/new", get(new_page))
        .route("/workflows/{id}", get(edit_page).post(update))
        .route("/workflows/{id}/toggle", post(toggle))
        .route("/workflows/{id}/delete", post(delete))
        .route("/workflows/{id}/test", post(test))
}

struct Choice {
    value: String,
    label: String,
    selected: bool,
}

struct Notice {
    danger: bool,
    lines: Vec<String>,
}

struct RunView {
    event: String,
    document: String,
    status_class: &'static str,
    status_label: &'static str,
    attempts: u32,
    response_status: Option<u16>,
    error: Option<String>,
    created: String,
}

impl From<&WorkflowRun> for RunView {
    fn from(run: &WorkflowRun) -> Self {
        let (status_class, status_label) = match run.status {
            RunStatus::Succeeded => ("f-status-done", "Delivered"),
            RunStatus::Pending if run.attempts > 0 => ("f-status-pending", "Retrying"),
            RunStatus::Pending => ("f-status-pending", "Waiting"),
            RunStatus::Failed => ("f-status-failed", "Failed"),
        };
        Self {
            event: run.event.as_str().to_string(),
            document: run
                .document_id
                .map(|id| id.to_string()[..8].to_string())
                .unwrap_or_default(),
            status_class,
            status_label,
            attempts: run.attempts,
            response_status: run.response_status,
            error: run.last_error.clone(),
            created: run.created_at.format("%Y-%m-%d %H:%M").to_string(),
        }
    }
}

struct WorkflowRow {
    id: String,
    name: String,
    enabled: bool,
    trigger: &'static str,
    last_run: Option<RunView>,
}

#[derive(Template)]
#[template(path = "workflows.html")]
struct ListTemplate {
    title: String,
    rows: Vec<WorkflowRow>,
}

async fn list_page(State(state): State<AppState>) -> ApiResult<Html<String>> {
    let mut rows = Vec::new();
    for workflow in state.workflow_repo.list().await? {
        let last_run = state
            .workflow_repo
            .list_runs(workflow.id, 1)
            .await?
            .first()
            .map(RunView::from);
        rows.push(WorkflowRow {
            id: workflow.id.to_string(),
            name: workflow.name,
            enabled: workflow.enabled,
            trigger: workflow.trigger.as_str(),
            last_run,
        });
    }
    render(ListTemplate {
        title: "Workflows - Folio".into(),
        rows,
    })
}

/// What the form shows, whether it came from a stored workflow or a rejected submission
#[derive(Default, Deserialize, Clone)]
#[serde(default)]
struct FormValues {
    name: String,
    trigger: String,
    enabled: Option<String>,
    mime_type: String,
    tag_id: String,
    correspondent_id: String,
    document_type_id: String,
    filename_glob: String,
    webhook_url: String,
    webhook_secret: String,
    clear_secret: Option<String>,
    webhook_headers: String,
}

impl FormValues {
    fn blank() -> Self {
        Self {
            trigger: WorkflowTrigger::DocumentUploaded.as_str().into(),
            enabled: Some("on".into()),
            ..Default::default()
        }
    }

    fn from_workflow(workflow: &Workflow) -> Self {
        let c = &workflow.conditions;
        let (webhook_url, webhook_headers) = match first_webhook(&workflow.actions) {
            Some((_, WorkflowAction::Webhook { url, headers, .. })) => (
                url.clone(),
                headers
                    .iter()
                    .map(|(k, v)| {
                        let shown = if v.is_empty() { "" } else { SECRET_MASK };
                        format!("{k}: {shown}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => Default::default(),
        };
        Self {
            name: workflow.name.clone(),
            trigger: workflow.trigger.as_str().into(),
            enabled: workflow.enabled.then(|| "on".to_string()),
            mime_type: c.mime_type.clone().unwrap_or_default(),
            tag_id: c.tag_id.map(|i| i.to_string()).unwrap_or_default(),
            correspondent_id: c
                .correspondent_id
                .map(|i| i.to_string())
                .unwrap_or_default(),
            document_type_id: c
                .document_type_id
                .map(|i| i.to_string())
                .unwrap_or_default(),
            filename_glob: c.filename_glob.clone().unwrap_or_default(),
            webhook_url,
            webhook_headers,
            ..Default::default()
        }
    }

    /// Turn the submission into a workflow, keeping any actions the form cannot edit
    fn into_input(self, existing: Option<&Workflow>) -> Result<WorkflowInput, String> {
        let trigger = WorkflowTrigger::parse(&self.trigger).ok_or("Choose a trigger.")?;
        let optional = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
        let id = |s: &str, what: &str| -> Result<Option<Id>, String> {
            optional(s)
                .map(|v| Uuid::parse_str(&v).map_err(|_| format!("The {what} is not valid.")))
                .transpose()
        };
        let conditions = WorkflowConditions {
            mime_type: optional(&self.mime_type),
            tag_id: id(&self.tag_id, "tag")?,
            correspondent_id: id(&self.correspondent_id, "correspondent")?,
            document_type_id: id(&self.document_type_id, "document type")?,
            filename_glob: optional(&self.filename_glob),
        };

        let mut headers = BTreeMap::new();
        for line in self
            .webhook_headers
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| format!("Header '{line}' needs the form Name: value."))?;
            headers.insert(name.trim().to_string(), value.trim().to_string());
        }
        // Masked values from the edit form stand for the stored ones.
        let new_url = self.webhook_url.trim();
        let stored_hook = match existing.and_then(|w| first_webhook(&w.actions)) {
            Some((
                _,
                WorkflowAction::Webhook {
                    url,
                    headers,
                    secret,
                    ..
                },
            )) if url == new_url => Some((headers, secret)),
            _ => None,
        };
        if let Some((stored, _)) = stored_hook {
            for (name, value) in headers
                .iter_mut()
                .filter(|(_, v)| v.as_str() == SECRET_MASK)
            {
                *value = stored
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("Header {name} has no stored value; enter it again."))?;
            }
        } else if headers.values().any(|v| v == SECRET_MASK) {
            return Err("Enter the header values; the mask only keeps stored ones.".into());
        }

        let mut actions = existing.map(|w| w.actions.clone()).unwrap_or_default();
        let stored = first_webhook(&actions).map(|(index, _)| index);
        // A stored secret is only kept while the URL stays the same, so it
        // cannot be sent to a new destination by editing the URL alone.
        let secret = if self.clear_secret.is_some() {
            None
        } else if let Some(secret) = optional(&self.webhook_secret) {
            Some(secret)
        } else {
            stored_hook.and_then(|(_, secret)| secret.clone())
        };
        let webhook = WorkflowAction::Webhook {
            url: self.webhook_url.trim().to_string(),
            method: WebhookMethod::Post,
            headers,
            secret,
        };
        match stored {
            Some(index) => actions[index] = webhook,
            None => actions.insert(0, webhook),
        }

        Ok(WorkflowInput {
            name: self.name.trim().to_string(),
            enabled: self.enabled.is_some(),
            trigger,
            conditions,
            actions,
        })
    }
}

fn first_webhook(actions: &[WorkflowAction]) -> Option<(usize, &WorkflowAction)> {
    actions.iter().enumerate().find(|(_, a)| a.is_webhook())
}

#[derive(Template)]
#[template(path = "workflow_form.html")]
struct FormTemplate {
    title: String,
    workflow_id: Option<String>,
    values: FormValues,
    has_secret: bool,
    secret_mask: &'static str,
    triggers: Vec<Choice>,
    tags: Vec<Choice>,
    correspondents: Vec<Choice>,
    document_types: Vec<Choice>,
    runs: Vec<RunView>,
    notice: Option<Notice>,
}

async fn build_form(
    state: &AppState,
    workflow: Option<&Workflow>,
    values: FormValues,
    notice: Option<Notice>,
) -> ApiResult<FormTemplate> {
    let choices = |items: Vec<(String, String)>, current: &str| {
        items
            .into_iter()
            .map(|(value, label)| Choice {
                selected: value == current,
                value,
                label,
            })
            .collect::<Vec<_>>()
    };
    let tags = state.tag_repo.list().await?;
    let correspondents = state.correspondent_repo.list().await?;
    let document_types = state.document_type_repo.list().await?;
    let runs = match workflow {
        Some(w) => state
            .workflow_repo
            .list_runs(w.id, RUN_HISTORY)
            .await?
            .iter()
            .map(RunView::from)
            .collect(),
        None => Vec::new(),
    };
    let has_secret = workflow
        .and_then(|w| first_webhook(&w.actions))
        .is_some_and(
            |(_, a)| matches!(a, WorkflowAction::Webhook { secret: Some(s), .. } if !s.is_empty()),
        );

    Ok(FormTemplate {
        title: format!(
            "{} - Folio",
            if workflow.is_some() {
                "Edit workflow"
            } else {
                "New workflow"
            }
        ),
        workflow_id: workflow.map(|w| w.id.to_string()),
        triggers: choices(
            WorkflowTrigger::ALL
                .iter()
                .map(|t| (t.as_str().to_string(), t.as_str().to_string()))
                .collect(),
            &values.trigger,
        ),
        tags: choices(
            tags.into_iter()
                .map(|t| (t.id.to_string(), t.name))
                .collect(),
            &values.tag_id,
        ),
        correspondents: choices(
            correspondents
                .into_iter()
                .map(|c| (c.id.to_string(), c.name))
                .collect(),
            &values.correspondent_id,
        ),
        document_types: choices(
            document_types
                .into_iter()
                .map(|d| (d.id.to_string(), d.name))
                .collect(),
            &values.document_type_id,
        ),
        values,
        has_secret,
        secret_mask: SECRET_MASK,
        runs,
        notice,
    })
}

fn danger(text: impl Into<String>) -> Option<Notice> {
    Some(Notice {
        danger: true,
        lines: vec![text.into()],
    })
}

async fn new_page(State(state): State<AppState>) -> ApiResult<Html<String>> {
    render(build_form(&state, None, FormValues::blank(), None).await?)
}

async fn edit_page(State(state): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Html<String>> {
    let workflow = find(&*state.workflow_repo, id).await?;
    let values = FormValues::from_workflow(&workflow);
    render(build_form(&state, Some(&workflow), values, None).await?)
}

async fn create(
    State(state): State<AppState>,
    Form(values): Form<FormValues>,
) -> ApiResult<Response> {
    let saved = match values.clone().into_input(None) {
        Ok(input) => state
            .workflow_repo
            .create(input)
            .await
            .map_err(|e| e.to_string()),
        Err(message) => Err(message),
    };
    match saved {
        Ok(workflow) => Ok(Redirect::to(&format!("/workflows/{}", workflow.id)).into_response()),
        Err(message) => {
            Ok(render(build_form(&state, None, values, danger(message)).await?)?.into_response())
        }
    }
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(values): Form<FormValues>,
) -> ApiResult<Response> {
    let workflow = find(&*state.workflow_repo, id).await?;
    let saved = match values.clone().into_input(Some(&workflow)) {
        Ok(input) => state
            .workflow_repo
            .update(id, input)
            .await
            .map_err(|e| e.to_string()),
        Err(message) => Err(message),
    };
    match saved {
        Ok(_) => Ok(Redirect::to(&format!("/workflows/{id}")).into_response()),
        Err(message) => Ok(render(
            build_form(&state, Some(&workflow), values, danger(message)).await?,
        )?
        .into_response()),
    }
}

async fn toggle(State(state): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Redirect> {
    let workflow = find(&*state.workflow_repo, id).await?;
    state
        .workflow_repo
        .update(
            id,
            WorkflowInput {
                name: workflow.name,
                enabled: !workflow.enabled,
                trigger: workflow.trigger,
                conditions: workflow.conditions,
                actions: workflow.actions,
            },
        )
        .await?;
    Ok(Redirect::to("/workflows"))
}

async fn delete(State(state): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Redirect> {
    state.workflow_repo.delete(id).await?;
    Ok(Redirect::to("/workflows"))
}

async fn test(State(state): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<Html<String>> {
    let workflow = find(&*state.workflow_repo, id).await?;
    let notice = match send_test(&state.webhook_client, &workflow, &state.public_url).await {
        Ok(result) => Notice {
            danger: result.deliveries.iter().any(|d| !d.success),
            lines: result
                .deliveries
                .iter()
                .map(|d| match (&d.error, d.status) {
                    (None, Some(status)) => {
                        format!(
                            "Delivered to {} (HTTP {status}, {} ms).",
                            d.url, d.duration_ms
                        )
                    }
                    (Some(error), _) => format!("Could not deliver to {}: {error}", d.url),
                    (None, None) => format!("Delivered to {}.", d.url),
                })
                .collect(),
        },
        Err(e) => Notice {
            danger: true,
            lines: vec![e.message().to_string()],
        },
    };
    let values = FormValues::from_workflow(&workflow);
    render(build_form(&state, Some(&workflow), values, Some(notice)).await?)
}

fn render<T: Template>(template: T) -> ApiResult<Html<String>> {
    template
        .render()
        .map(Html)
        .map_err(|e| ApiError::internal(format!("Failed to render page: {e}")))
}
