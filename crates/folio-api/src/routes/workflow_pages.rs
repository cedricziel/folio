//! Server-rendered workflow pages: list, create/edit form, test, and run history

use crate::vault::filters;
use std::collections::{BTreeMap, HashMap};

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use folio_core::models::{
    Id, RunStatus, SourceConditions, SourceKind, TextMatch, WebhookMethod, Workflow,
    WorkflowAction, WorkflowConditions, WorkflowInput, WorkflowRun, WorkflowTrigger,
};

use super::workflows::{find, find_retained, send_test, WorkflowListQuery, SECRET_MASK};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::vault::DefaultVault;

use super::view::{Nav, Sidebar};

const RUN_HISTORY: u32 = 20;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/workflows", get(list_page).post(create))
        .route("/workflows/new", get(new_page))
        .route("/workflows/{id}", get(edit_page).post(update))
        .route("/workflows/{id}/toggle", post(toggle))
        .route("/workflows/{id}/delete", post(delete))
        .route("/workflows/{id}/restore", post(restore))
        .route("/workflows/{id}/history", get(history))
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
    event: &'static str,
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
            event: run.event.label(),
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
    lifecycle_summary: String,
    id: String,
    name: String,
    enabled: bool,
    trigger: &'static str,
    last_run: Option<RunView>,
}

#[derive(Template)]
#[template(path = "workflows.html")]
struct ListTemplate {
    deleted: bool,
    sidebar: Sidebar,
    title: String,
    rows: Vec<WorkflowRow>,
}

async fn list_page(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Query(query): Query<WorkflowListQuery>,
) -> ApiResult<Html<String>> {
    let mut rows = Vec::new();
    let workflows = if query.deleted {
        state.workflow_repo.list_deleted(vault).await?
    } else {
        state.workflow_repo.list(vault).await?
    };
    for workflow in workflows {
        let last_run = state
            .workflow_repo
            .list_runs(vault, workflow.id, 1)
            .await?
            .first()
            .map(RunView::from);
        rows.push(WorkflowRow {
            lifecycle_summary: lifecycle_summary(&workflow),
            id: workflow.id.to_string(),
            name: workflow.name,
            enabled: workflow.enabled,
            trigger: workflow.trigger.label(),
            last_run,
        });
    }
    render(ListTemplate {
        deleted: query.deleted,
        sidebar: Sidebar::load(&state, vault, Nav::Workflows).await?,
        title: "Workflows - Folio".into(),
        rows,
    })
}

fn lifecycle_summary(workflow: &Workflow) -> String {
    match workflow.deleted_at {
        Some(at) => format!("Deleted {}", at.format("%Y-%m-%d %H:%M UTC")),
        None => format!(
            "Created {}, updated {}",
            workflow.created_at.format("%Y-%m-%d %H:%M UTC"),
            workflow.updated_at.format("%Y-%m-%d %H:%M UTC")
        ),
    }
}

#[derive(Template)]
#[template(path = "workflow_history.html")]
struct HistoryTemplate {
    sidebar: Sidebar,
    title: String,
    id: String,
    name: String,
    deleted: bool,
    lifecycle_summary: String,
    runs: Vec<RunView>,
}

async fn history(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Html<String>> {
    let workflow = find_retained(vault, &*state.workflow_repo, id).await?;
    let runs = state
        .workflow_repo
        .list_runs(vault, id, RUN_HISTORY)
        .await?;
    render(HistoryTemplate {
        sidebar: Sidebar::load(&state, vault, Nav::Workflows).await?,
        title: format!("{} history - Folio", workflow.name),
        id: id.to_string(),
        deleted: workflow.deleted_at.is_some(),
        lifecycle_summary: lifecycle_summary(&workflow),
        name: workflow.name,
        runs: runs.iter().map(RunView::from).collect(),
    })
}

async fn restore(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Redirect> {
    state.workflow_repo.restore(vault, id).await?;
    Ok(Redirect::to("/workflows"))
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
    source_kind: String,
    folder_path_glob: String,
    email_from_mode: String,
    email_from: String,
    email_to_mode: String,
    email_to: String,
    email_subject_mode: String,
    email_subject: String,
    email_attachment_glob: String,
    email_mailbox_glob: String,
    webhook_url: String,
    webhook_secret: String,
    clear_secret: Option<String>,
    webhook_headers: String,
    action_correspondent_id: String,
    action_document_type_id: String,
    /// `action_tag_<id>` checkboxes: the urlencoded form cannot repeat a key
    #[serde(flatten)]
    checkboxes: HashMap<String, String>,
}

const TAG_FIELD: &str = "action_tag_";

impl FormValues {
    fn action_tag_ids(&self) -> Vec<Id> {
        self.checkboxes
            .keys()
            .filter_map(|k| k.strip_prefix(TAG_FIELD))
            .filter_map(|id| Uuid::parse_str(id).ok())
            .collect()
    }

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
        let mut checkboxes = HashMap::new();
        let (mut action_correspondent_id, mut action_document_type_id) = Default::default();
        for action in &workflow.actions {
            match action {
                WorkflowAction::AddTags { tag_ids } => checkboxes.extend(
                    tag_ids
                        .iter()
                        .map(|id| (format!("{TAG_FIELD}{id}"), "on".to_string())),
                ),
                WorkflowAction::SetCorrespondent { correspondent_id } => {
                    action_correspondent_id = correspondent_id.to_string()
                }
                WorkflowAction::SetDocumentType { document_type_id } => {
                    action_document_type_id = document_type_id.to_string()
                }
                WorkflowAction::Webhook { .. } | WorkflowAction::SkipImport => {}
            }
        }
        Self {
            checkboxes,
            action_correspondent_id,
            action_document_type_id,
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
            source_kind: c
                .source
                .kind
                .map(|k| k.as_str().to_string())
                .unwrap_or_default(),
            folder_path_glob: c.source.folder_path_glob.clone().unwrap_or_default(),
            email_from_mode: mode_of(&c.source.email_from),
            email_from: text_of(&c.source.email_from),
            email_to_mode: mode_of(&c.source.email_to),
            email_to: text_of(&c.source.email_to),
            email_subject_mode: mode_of(&c.source.email_subject),
            email_subject: text_of(&c.source.email_subject),
            email_attachment_glob: c.source.email_attachment_glob.clone().unwrap_or_default(),
            email_mailbox_glob: c.source.email_mailbox_glob.clone().unwrap_or_default(),
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
            source: SourceConditions {
                kind: optional(&self.source_kind).map(|k| SourceKind::parse(&k)),
                folder_path_glob: optional(&self.folder_path_glob),
                email_from: text_match(&self.email_from_mode, &self.email_from),
                email_to: text_match(&self.email_to_mode, &self.email_to),
                email_subject: text_match(&self.email_subject_mode, &self.email_subject),
                email_attachment_glob: optional(&self.email_attachment_glob),
                email_mailbox_glob: optional(&self.email_mailbox_glob),
            },
            ..existing
                .map(|workflow| workflow.conditions.clone())
                .unwrap_or_default()
        };

        if trigger == WorkflowTrigger::IngestReceived {
            return Ok(WorkflowInput {
                name: self.name.trim().to_string(),
                enabled: self.enabled.is_some(),
                trigger,
                conditions,
                actions: vec![WorkflowAction::SkipImport],
            });
        }
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

        let mut tag_ids = self.action_tag_ids();
        tag_ids.sort();
        // A stored secret is only kept while the URL stays the same, so it
        // cannot be sent to a new destination by editing the URL alone.
        let secret = if self.clear_secret.is_some() {
            None
        } else if let Some(secret) = optional(&self.webhook_secret) {
            Some(secret)
        } else {
            stored_hook.and_then(|(_, secret)| secret.clone())
        };

        // The form edits one action of each kind: it replaces the first stored
        // one, or removes it when the section was emptied. Anything else stays.
        let mut actions = existing.map(|w| w.actions.clone()).unwrap_or_default();
        set_action(
            &mut actions,
            |a| matches!(a, WorkflowAction::AddTags { .. }),
            (!tag_ids.is_empty()).then_some(WorkflowAction::AddTags { tag_ids }),
        );
        set_action(
            &mut actions,
            |a| matches!(a, WorkflowAction::SetCorrespondent { .. }),
            id(&self.action_correspondent_id, "correspondent")?
                .map(|correspondent_id| WorkflowAction::SetCorrespondent { correspondent_id }),
        );
        set_action(
            &mut actions,
            |a| matches!(a, WorkflowAction::SetDocumentType { .. }),
            id(&self.action_document_type_id, "document type")?
                .map(|document_type_id| WorkflowAction::SetDocumentType { document_type_id }),
        );
        set_action(
            &mut actions,
            WorkflowAction::is_webhook,
            (!new_url.is_empty()).then(|| WorkflowAction::Webhook {
                url: new_url.to_string(),
                method: WebhookMethod::Post,
                headers,
                secret,
            }),
        );
        if actions.is_empty() {
            return Err("Choose what Folio should do: tag, file or send a webhook.".to_string());
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

fn set_action(
    actions: &mut Vec<WorkflowAction>,
    is_kind: fn(&WorkflowAction) -> bool,
    wanted: Option<WorkflowAction>,
) {
    match (actions.iter().position(is_kind), wanted) {
        (Some(i), Some(action)) => actions[i] = action,
        (Some(i), None) => {
            actions.remove(i);
        }
        (None, Some(action)) => actions.push(action),
        (None, None) => {}
    }
}

fn mode_of(m: &Option<TextMatch>) -> String {
    match m {
        Some(TextMatch::Contains(_)) => "contains",
        Some(TextMatch::Regex(_)) => "regex",
        _ => "glob",
    }
    .to_string()
}

fn text_of(m: &Option<TextMatch>) -> String {
    match m {
        Some(TextMatch::Glob(t) | TextMatch::Contains(t) | TextMatch::Regex(t)) => t.clone(),
        None => String::new(),
    }
}

fn text_match(mode: &str, value: &str) -> Option<TextMatch> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return None;
    }
    Some(match mode {
        "contains" => TextMatch::Contains(value),
        "regex" => TextMatch::Regex(value),
        _ => TextMatch::Glob(value),
    })
}

fn first_webhook(actions: &[WorkflowAction]) -> Option<(usize, &WorkflowAction)> {
    actions.iter().enumerate().find(|(_, a)| a.is_webhook())
}

#[derive(Template)]
#[template(path = "workflow_form.html")]
struct FormTemplate {
    sidebar: Sidebar,
    title: String,
    workflow_id: Option<String>,
    values: FormValues,
    has_secret: bool,
    secret_mask: &'static str,
    triggers: Vec<Choice>,
    tags: Vec<Choice>,
    correspondents: Vec<Choice>,
    document_types: Vec<Choice>,
    source_kinds: Vec<Choice>,
    action_tags: Vec<Choice>,
    action_correspondents: Vec<Choice>,
    action_document_types: Vec<Choice>,
    show_folder: bool,
    show_email: bool,
    runs: Vec<RunView>,
    notice: Option<Notice>,
}

async fn build_form(
    DefaultVault(vault): DefaultVault,
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
    let chosen_tags = values.action_tag_ids();
    let pairs = |names: Vec<(Id, String)>| -> Vec<(String, String)> {
        names
            .into_iter()
            .map(|(id, name)| (id.to_string(), name))
            .collect()
    };
    let tags = state.tag_repo.list(vault).await?;
    let correspondents = state.correspondent_repo.list(vault).await?;
    let document_types = state.document_type_repo.list(vault).await?;
    let runs = match workflow {
        Some(w) => state
            .workflow_repo
            .list_runs(vault, w.id, RUN_HISTORY)
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
        sidebar: Sidebar::load(state, vault, Nav::Workflows).await?,
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
                .map(|t| (t.as_str().to_string(), t.label().to_string()))
                .collect(),
            &values.trigger,
        ),
        source_kinds: choices(
            [
                ("upload", "Uploaded in the browser"),
                ("api", "Sent through the API"),
                ("folder", "A watched folder"),
                ("email", "A mail account"),
            ]
            .map(|(value, label)| (value.to_string(), label.to_string()))
            .to_vec(),
            &values.source_kind,
        ),
        action_tags: tags
            .iter()
            .map(|t| Choice {
                selected: chosen_tags.contains(&t.id),
                value: t.id.to_string(),
                label: t.name.clone(),
            })
            .collect(),
        action_correspondents: choices(
            pairs(
                correspondents
                    .iter()
                    .map(|c| (c.id, c.name.clone()))
                    .collect(),
            ),
            &values.action_correspondent_id,
        ),
        action_document_types: choices(
            pairs(
                document_types
                    .iter()
                    .map(|d| (d.id, d.name.clone()))
                    .collect(),
            ),
            &values.action_document_type_id,
        ),
        show_folder: values.source_kind == "folder" || !values.folder_path_glob.is_empty(),
        show_email: values.source_kind == "email"
            || [
                &values.email_from,
                &values.email_to,
                &values.email_subject,
                &values.email_attachment_glob,
                &values.email_mailbox_glob,
            ]
            .iter()
            .any(|v| !v.is_empty()),
        tags: choices(
            pairs(tags.iter().map(|t| (t.id, t.name.clone())).collect()),
            &values.tag_id,
        ),
        correspondents: choices(
            pairs(
                correspondents
                    .iter()
                    .map(|c| (c.id, c.name.clone()))
                    .collect(),
            ),
            &values.correspondent_id,
        ),
        document_types: choices(
            pairs(
                document_types
                    .iter()
                    .map(|d| (d.id, d.name.clone()))
                    .collect(),
            ),
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

async fn new_page(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
) -> ApiResult<Html<String>> {
    render(build_form(DefaultVault(vault), &state, None, FormValues::blank(), None).await?)
}

async fn edit_page(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Html<String>> {
    let workflow = find(vault, &*state.workflow_repo, id).await?;
    let values = FormValues::from_workflow(&workflow);
    render(build_form(DefaultVault(vault), &state, Some(&workflow), values, None).await?)
}

async fn create(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Form(values): Form<FormValues>,
) -> ApiResult<Response> {
    let saved = match values.clone().into_input(None) {
        Ok(input) => state
            .workflow_repo
            .create(vault, input)
            .await
            .map_err(|e| e.to_string()),
        Err(message) => Err(message),
    };
    match saved {
        Ok(workflow) => Ok(Redirect::to(&format!("/workflows/{}", workflow.id)).into_response()),
        Err(message) => Ok(render(
            build_form(DefaultVault(vault), &state, None, values, danger(message)).await?,
        )?
        .into_response()),
    }
}

async fn update(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(values): Form<FormValues>,
) -> ApiResult<Response> {
    let workflow = find(vault, &*state.workflow_repo, id).await?;
    let saved = match values.clone().into_input(Some(&workflow)) {
        Ok(input) => state
            .workflow_repo
            .update(vault, id, input)
            .await
            .map_err(|e| e.to_string()),
        Err(message) => Err(message),
    };
    match saved {
        Ok(_) => Ok(Redirect::to(&format!("/workflows/{id}")).into_response()),
        Err(message) => Ok(render(
            build_form(
                DefaultVault(vault),
                &state,
                Some(&workflow),
                values,
                danger(message),
            )
            .await?,
        )?
        .into_response()),
    }
}

async fn toggle(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Redirect> {
    let workflow = find(vault, &*state.workflow_repo, id).await?;
    state
        .workflow_repo
        .update(
            vault,
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

async fn delete(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Redirect> {
    state.workflow_repo.delete(vault, id).await?;
    Ok(Redirect::to("/workflows"))
}

async fn test(
    DefaultVault(vault): DefaultVault,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(values): Form<FormValues>,
) -> ApiResult<Html<String>> {
    let workflow = find(vault, &*state.workflow_repo, id).await?;
    let sent = match values.clone().into_input(Some(&workflow)) {
        Ok(draft) => send_test(
            &state.webhook_client,
            draft.trigger,
            &draft.actions,
            &state.public_url,
        )
        .await
        .map_err(|e| e.message().to_string()),
        Err(message) => Err(message),
    };
    let notice = match sent {
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
        Err(message) => Notice {
            danger: true,
            lines: vec![message],
        },
    };
    render(
        build_form(
            DefaultVault(vault),
            &state,
            Some(&workflow),
            values,
            Some(notice),
        )
        .await?,
    )
}

fn render<T: Template>(template: T) -> ApiResult<Html<String>> {
    template
        .render()
        .map(Html)
        .map_err(|e| ApiError::internal(format!("Failed to render page: {e}")))
}
