//! Server-rendered workflow pages: list, create/edit form, test, and run history

use crate::vault::filters;
use std::collections::{BTreeSet, HashMap};

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use chrono::SecondsFormat;
use folio_core::Error;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use folio_core::models::{
    Id, MetadataChangeState, MetadataKind, MetadataValue, RunStatus, SourceConditions, SourceKind,
    TextMatch, UserId, WebhookMethod, Workflow, WorkflowAction, WorkflowActorCondition,
    WorkflowCadence, WorkflowConditionField, WorkflowConditionType, WorkflowConditions,
    WorkflowDocumentConditions, WorkflowEventConditions, WorkflowIngestConditions, WorkflowInput,
    WorkflowMetadataConditions, WorkflowProcessingConditions, WorkflowRun, WorkflowTarget,
    WorkflowTrigger, WorkflowTriggerRegistry,
};

use super::workflow_action_form::{action_fields, parse_headers, resolve_actions};
use super::workflows::{
    find, find_retained, history_error, send_test, WorkflowListQuery, SECRET_MASK,
};
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

struct TriggerChoice {
    value: String,
    label: String,
    selected: bool,
    processing_keys: String,
    document_actions: bool,
}

struct AdditionalTrigger {
    position: usize,
    choices: Vec<TriggerChoice>,
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
    completed_actions: u32,
    response_status: Option<u16>,
    error: Option<String>,
    created: String,
    finished: Option<String>,
}

impl From<&WorkflowRun> for RunView {
    fn from(run: &WorkflowRun) -> Self {
        let (status_class, status_label) = match run.status {
            RunStatus::Succeeded => ("f-status-done", "Succeeded"),
            RunStatus::Pending if run.attempts > 0 => ("f-status-pending", "Retrying"),
            RunStatus::Pending => ("f-status-pending", "Waiting"),
            RunStatus::Failed => ("f-status-failed", "Failed"),
        };
        let error = history_error(run).filter(|error| {
            run.response_status
                .is_none_or(|status| *error != format!("HTTP {status}"))
        });
        Self {
            event: run.event.label(),
            document: run
                .document_id
                .map(|id| id.to_string()[..8].to_string())
                .unwrap_or_default(),
            status_class,
            status_label,
            attempts: run.attempts,
            completed_actions: run.completed_actions,
            response_status: run.response_status,
            error,
            created: run.created_at.format("%Y-%m-%d %H:%M").to_string(),
            finished: run
                .finished_at
                .map(|at| at.format("%Y-%m-%d %H:%M").to_string()),
        }
    }
}

struct WorkflowRow {
    lifecycle_summary: String,
    id: String,
    name: String,
    enabled: bool,
    trigger_labels: Vec<&'static str>,
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
            trigger_labels: workflow
                .triggers
                .iter()
                .map(WorkflowTrigger::label)
                .collect(),
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
    let created_by = workflow
        .created_by
        .as_ref()
        .map(|actor| format!(" by {actor}"))
        .unwrap_or_else(|| " by System or unknown".into());
    let updated_by = workflow
        .updated_by
        .as_ref()
        .map(|actor| format!(" by {actor}"))
        .unwrap_or_else(|| " by System or unknown".into());
    let mut summary = format!(
        "Created {}{created_by}, updated {}{updated_by}",
        workflow.created_at.format("%Y-%m-%d %H:%M UTC"),
        workflow.updated_at.format("%Y-%m-%d %H:%M UTC")
    );
    if let Some(at) = workflow.deleted_at {
        summary.push_str(&format!(", deleted {}", at.format("%Y-%m-%d %H:%M UTC")));
    }
    summary
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
    revision: Option<i64>,
    ordered_actions: Option<String>,
    name: String,
    trigger: String,
    trigger_selection: Option<String>,
    cadence_mode: String,
    cadence_start: String,
    cadence_interval: String,
    clear_clock_filters: Option<String>,
    clear_clock_actions: Option<String>,
    enabled: Option<String>,
    condition_groups: Option<String>,
    mime_type: String,
    tag_id: String,
    correspondent_id: String,
    document_type_id: String,
    filename_glob: String,
    event_subject_id: String,
    event_actor: Option<String>,
    event_user_id: String,
    ingest_source_id: String,
    ingest_source_kind: Option<String>,
    ingest_reason: String,
    ingest_retryable: String,
    metadata_field_id: String,
    metadata_kind: String,
    metadata_state: String,
    metadata_before_enabled: Option<String>,
    metadata_before: String,
    metadata_before_currency: String,
    metadata_after_enabled: Option<String>,
    metadata_after: String,
    metadata_after_currency: String,
    title_before_enabled: Option<String>,
    title_before: String,
    title_after_enabled: Option<String>,
    title_after: String,
    added_tag_id: String,
    removed_tag_id: String,
    tag_change_state: String,
    assignment_before_id: String,
    assignment_after_id: String,
    assignment_change_state: String,
    document_date_before: String,
    document_date_after: String,
    document_date_state: String,
    review_origin: String,
    unreview_reason: String,
    revision_number: String,
    revision_source: String,
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
    /// Indexed triggers, tag selections, and processing predicates use distinct form keys.
    #[serde(flatten)]
    extra_fields: HashMap<String, String>,
}

fn processing_fields() -> impl Iterator<Item = &'static WorkflowConditionField> {
    let mut seen = BTreeSet::new();
    WorkflowTriggerRegistry::built_in()
        .definitions()
        .iter()
        .filter(|definition| definition.event_type.starts_with("document.processing."))
        .flat_map(|definition| &definition.condition_fields)
        .filter(move |field| seen.insert(field.key.as_str()))
}

fn processing_key(field: &WorkflowConditionField) -> &str {
    field.key.strip_prefix("data.").unwrap_or(&field.key)
}

fn processing_condition(
    form: &FormValues,
    existing: Option<&Workflow>,
    trigger: WorkflowTrigger,
) -> Result<Option<WorkflowProcessingConditions>, String> {
    if !form.extra_fields.contains_key("processing_form") {
        return Ok(existing
            .filter(|_| trigger.is_processing_outcome())
            .and_then(|workflow| workflow.conditions.event.as_ref())
            .and_then(|event| event.processing.clone()));
    }
    let mut filters = serde_json::Map::new();
    for field in processing_fields() {
        let key = processing_key(field);
        let Some(raw) = form
            .extra_fields
            .get(&format!("processing_{key}"))
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let value = match field.value_type {
            WorkflowConditionType::Integer => serde_json::Value::from(
                raw.parse::<u32>()
                    .map_err(|_| "Enter a valid processing attempt.")?,
            ),
            WorkflowConditionType::Boolean => serde_json::Value::Bool(
                raw.parse::<bool>()
                    .map_err(|_| "Choose whether text was found.")?,
            ),
            _ => serde_json::Value::String(raw.into()),
        };
        filters.insert(key.into(), value);
    }
    if filters.is_empty() {
        return Ok(None);
    }
    let processing: WorkflowProcessingConditions =
        serde_json::from_value(serde_json::Value::Object(filters))
            .map_err(|_| "Check the values in the processing filters.")?;
    processing
        .validate_trigger(trigger)
        .map_err(|error| match error {
            folio_core::Error::Validation(message) => message,
            other => other.to_string(),
        })?;
    Ok(Some(processing))
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct ConditionGroups {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    all: Vec<WorkflowConditions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    any: Option<Vec<WorkflowConditions>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    none: Vec<WorkflowConditions>,
}

impl ConditionGroups {
    fn from_conditions(conditions: &WorkflowConditions) -> Self {
        Self {
            all: conditions.all.clone(),
            any: conditions.any.clone(),
            none: conditions.none.clone(),
        }
    }

    fn apply(self, conditions: &mut WorkflowConditions) {
        conditions.all = self.all;
        conditions.any = self.any;
        conditions.none = self.none;
    }
}

const TAG_FIELD: &str = "action_tag_";
const ADDITIONAL_TRIGGER_FIELD: &str = "trigger_additional_";

const METADATA_KINDS: [(MetadataKind, &str); 10] = [
    (MetadataKind::Text, "Text"),
    (MetadataKind::LongText, "Long text"),
    (MetadataKind::Boolean, "Yes or no"),
    (MetadataKind::Integer, "Whole number"),
    (MetadataKind::Decimal, "Decimal"),
    (MetadataKind::Date, "Date"),
    (MetadataKind::Money, "Money"),
    (MetadataKind::Select, "Select"),
    (MetadataKind::Url, "URL"),
    (MetadataKind::DocumentLink, "Document link"),
];

fn metadata_value_text(value: Option<&MetadataValue>) -> (String, String) {
    match value {
        Some(MetadataValue::Money { amount, currency }) => (amount.clone(), currency.clone()),
        Some(MetadataValue::Boolean(value)) => (value.to_string(), String::new()),
        Some(MetadataValue::Select(id) | MetadataValue::DocumentLink(id)) => {
            (id.to_string(), String::new())
        }
        Some(
            MetadataValue::Text(value)
            | MetadataValue::LongText(value)
            | MetadataValue::Integer(value)
            | MetadataValue::Decimal(value)
            | MetadataValue::Date(value)
            | MetadataValue::Url(value),
        ) => (value.clone(), String::new()),
        None => (String::new(), String::new()),
    }
}

fn metadata_value(
    enabled: bool,
    kind: Option<MetadataKind>,
    value: &str,
    currency: &str,
) -> Result<Option<MetadataValue>, String> {
    if !enabled {
        return Ok(None);
    }
    let kind = kind.ok_or("Choose the field type for an exact value comparison.")?;
    let parse_id =
        || Uuid::parse_str(value.trim()).map_err(|_| "Enter a valid value ID.".to_string());
    let value = match kind {
        MetadataKind::Text => MetadataValue::Text(value.into()),
        MetadataKind::LongText => MetadataValue::LongText(value.into()),
        MetadataKind::Boolean => match value.trim() {
            "true" | "yes" => MetadataValue::Boolean(true),
            "false" | "no" => MetadataValue::Boolean(false),
            _ => return Err("Enter yes or no.".into()),
        },
        MetadataKind::Integer => MetadataValue::Integer(value.into()),
        MetadataKind::Decimal => MetadataValue::Decimal(value.into()),
        MetadataKind::Date => MetadataValue::Date(value.into()),
        MetadataKind::Money => MetadataValue::Money {
            amount: value.into(),
            currency: currency.into(),
        },
        MetadataKind::Select => MetadataValue::Select(parse_id()?),
        MetadataKind::Url => MetadataValue::Url(value.into()),
        MetadataKind::DocumentLink => MetadataValue::DocumentLink(parse_id()?),
    };
    value
        .validate(kind, false)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn metadata_condition(
    form: &FormValues,
    trigger: WorkflowTrigger,
) -> Result<Option<WorkflowMetadataConditions>, String> {
    if !trigger.is_custom_metadata() {
        return Ok(None);
    }
    let field_id = match form.metadata_field_id.as_str() {
        "" => None,
        id => Some(Uuid::parse_str(id).map_err(|_| "Choose a custom field.")?),
    };
    let kind = match form.metadata_kind.as_str() {
        "" => None,
        value => Some(
            METADATA_KINDS
                .iter()
                .find_map(|(kind, _)| (kind.as_str() == value).then_some(*kind))
                .ok_or("Choose a field type.")?,
        ),
    };
    let state = match form.metadata_state.as_str() {
        "" => None,
        "set" => Some(MetadataChangeState::Set),
        "replaced" => Some(MetadataChangeState::Replaced),
        "cleared" => Some(MetadataChangeState::Cleared),
        _ => return Err("Choose a change.".into()),
    };
    let conditions = WorkflowMetadataConditions {
        field_id,
        kind,
        state,
        before: metadata_value(
            form.metadata_before_enabled.is_some(),
            kind,
            &form.metadata_before,
            &form.metadata_before_currency,
        )?,
        after: metadata_value(
            form.metadata_after_enabled.is_some(),
            kind,
            &form.metadata_after,
            &form.metadata_after_currency,
        )?,
    };
    Ok((conditions != WorkflowMetadataConditions::default()).then_some(conditions))
}

fn condition_choice<T: serde::de::DeserializeOwned>(
    value: &str,
    label: &str,
) -> Result<Option<T>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_value(serde_json::Value::String(value.trim().into()))
        .map(Some)
        .map_err(|_| format!("The {label} is not valid."))
}

fn condition_value<T: serde::Serialize>(value: Option<T>) -> String {
    value
        .and_then(|value| serde_json::to_value(value).ok())
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

impl FormValues {
    fn document_condition(
        &self,
        trigger: WorkflowTrigger,
    ) -> Result<Option<WorkflowDocumentConditions>, String> {
        use WorkflowDocumentConditions::{
            Correspondent, Date, DocumentType, Review, Revision, Tags, Title,
        };
        let condition = match trigger {
            WorkflowTrigger::DocumentTitleChanged => Title {
                before: self
                    .title_before_enabled
                    .as_ref()
                    .map(|_| self.title_before.clone()),
                after: self
                    .title_after_enabled
                    .as_ref()
                    .map(|_| self.title_after.clone()),
            },
            WorkflowTrigger::DocumentTagsChanged => Tags {
                added_id: condition_choice(&self.added_tag_id, "added tag")?,
                removed_id: condition_choice(&self.removed_tag_id, "removed tag")?,
                state: condition_choice(&self.tag_change_state, "tag change")?,
            },
            WorkflowTrigger::DocumentCorrespondentChanged
            | WorkflowTrigger::DocumentTypeChanged => {
                let before_id =
                    condition_choice(&self.assignment_before_id, "previous classification")?;
                let after_id = condition_choice(&self.assignment_after_id, "new classification")?;
                let state =
                    condition_choice(&self.assignment_change_state, "classification change")?;
                if trigger == WorkflowTrigger::DocumentCorrespondentChanged {
                    Correspondent {
                        before_id,
                        after_id,
                        state,
                    }
                } else {
                    DocumentType {
                        before_id,
                        after_id,
                        state,
                    }
                }
            }
            WorkflowTrigger::DocumentDateChanged => Date {
                before: condition_choice(&self.document_date_before, "previous document date")?,
                after: condition_choice(&self.document_date_after, "new document date")?,
                state: condition_choice(&self.document_date_state, "date change")?,
            },
            WorkflowTrigger::DocumentReviewed | WorkflowTrigger::DocumentUnreviewed => Review {
                origin: condition_choice(&self.review_origin, "review origin")?,
                reason: if trigger == WorkflowTrigger::DocumentUnreviewed {
                    condition_choice(&self.unreview_reason, "unreview reason")?
                } else {
                    None
                },
            },
            WorkflowTrigger::DocumentRevisionCreated
            | WorkflowTrigger::DocumentRevisionRestored => Revision {
                number: if self.revision_number.trim().is_empty() {
                    None
                } else {
                    Some(
                        self.revision_number
                            .trim()
                            .parse()
                            .map_err(|_| "Enter a whole revision number.")?,
                    )
                },
                source: if trigger == WorkflowTrigger::DocumentRevisionCreated {
                    condition_choice(&self.revision_source, "revision source")?
                } else {
                    None
                },
            },
            _ => return Ok(None),
        };
        match condition {
            Title {
                before: None,
                after: None,
            }
            | Tags {
                added_id: None,
                removed_id: None,
                state: None,
            }
            | Correspondent {
                before_id: None,
                after_id: None,
                state: None,
            }
            | DocumentType {
                before_id: None,
                after_id: None,
                state: None,
            }
            | Date {
                before: None,
                after: None,
                state: None,
            }
            | Review {
                origin: None,
                reason: None,
            }
            | Revision {
                number: None,
                source: None,
            } => Ok(None),
            _ => {
                condition
                    .validate_trigger(trigger)
                    .map_err(|error| error.to_string())?;
                Ok(Some(condition))
            }
        }
    }

    fn ingest_kind_is(&self, kind: &str) -> bool {
        self.ingest_source_kind.as_deref() == Some(kind)
    }

    fn is_system_actor(&self) -> bool {
        self.event_actor.as_deref() == Some("system")
    }

    fn is_user_actor(&self) -> bool {
        self.event_actor.as_deref() == Some("user")
    }

    fn action_tag_ids(&self) -> Vec<Id> {
        self.extra_fields
            .keys()
            .filter_map(|k| k.strip_prefix(TAG_FIELD))
            .filter_map(|id| Uuid::parse_str(id).ok())
            .collect()
    }

    fn additional_triggers(&self) -> Vec<(usize, &str)> {
        (1..WorkflowTrigger::ALL.len())
            .filter_map(|position| {
                self.extra_fields
                    .get(&format!("{ADDITIONAL_TRIGGER_FIELD}{position}"))
                    .map(|value| (position, value.as_str()))
            })
            .collect()
    }

    fn explicit_triggers(&self, trigger: WorkflowTrigger) -> Result<Vec<WorkflowTrigger>, String> {
        if self.trigger_selection.is_none() {
            return Ok(Vec::new());
        }
        for suffix in self
            .extra_fields
            .keys()
            .filter_map(|key| key.strip_prefix(ADDITIONAL_TRIGGER_FIELD))
        {
            let position: usize = suffix
                .parse()
                .map_err(|_| "The additional trigger position is invalid.")?;
            if !(1..WorkflowTrigger::ALL.len()).contains(&position)
                || suffix != position.to_string()
            {
                return Err("The additional trigger position is invalid.".into());
            }
        }
        let mut triggers = vec![trigger];
        for (_, value) in self.additional_triggers() {
            if !value.is_empty() {
                triggers.push(
                    WorkflowTrigger::parse(value).ok_or("Choose a valid additional trigger.")?,
                );
            }
        }
        Ok(triggers)
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
        let (event_actor, event_user_id) =
            match c.event.as_ref().and_then(|event| event.actor.as_ref()) {
                Some(WorkflowActorCondition::System) => ("system", String::new()),
                Some(WorkflowActorCondition::User { user_id }) => {
                    ("user", user_id.as_str().to_owned())
                }
                None => ("", String::new()),
            };
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
        let mut extra_fields: HashMap<_, _> = workflow
            .triggers
            .iter()
            .enumerate()
            .skip(1)
            .map(|(position, trigger)| {
                (
                    format!("{ADDITIONAL_TRIGGER_FIELD}{position}"),
                    trigger.as_str().into(),
                )
            })
            .collect();
        let (mut action_correspondent_id, mut action_document_type_id) = Default::default();
        for action in &workflow.actions {
            match action {
                WorkflowAction::AddTags { tag_ids } => extra_fields.extend(
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
                _ => {}
            }
        }
        if let Some(processing) = c.event.as_ref().and_then(|event| event.processing.as_ref()) {
            if let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(processing) {
                extra_fields.extend(fields.into_iter().map(|(name, value)| {
                    let text = match value {
                        serde_json::Value::String(text) => text,
                        other => other.to_string(),
                    };
                    (format!("processing_{name}"), text)
                }));
            }
        }
        let metadata = c.event.as_ref().and_then(|event| event.metadata.as_ref());
        let before = metadata.and_then(|metadata| metadata.before.as_ref());
        let after = metadata.and_then(|metadata| metadata.after.as_ref());
        let (metadata_before, metadata_before_currency) = metadata_value_text(before);
        let (metadata_after, metadata_after_currency) = metadata_value_text(after);
        let (cadence_mode, cadence_start, cadence_interval) = match workflow.cadence {
            Some(WorkflowCadence::Once { at }) => (
                "once",
                at.to_rfc3339_opts(SecondsFormat::AutoSi, true),
                String::new(),
            ),
            Some(WorkflowCadence::Recurring {
                first_at,
                interval_seconds,
            }) => (
                "recurring",
                first_at.to_rfc3339_opts(SecondsFormat::AutoSi, true),
                interval_seconds.to_string(),
            ),
            None => ("once", String::new(), String::new()),
        };
        let mut values = Self {
            extra_fields,
            revision: Some(workflow.revision),
            action_correspondent_id,
            action_document_type_id,
            name: workflow.name.clone(),
            trigger: workflow.trigger.as_str().into(),
            cadence_mode: cadence_mode.into(),
            cadence_start,
            cadence_interval,
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
            event_subject_id: c
                .event
                .as_ref()
                .and_then(|event| event.subject_id)
                .map(|id| id.to_string())
                .unwrap_or_default(),
            event_actor: Some(event_actor.into()),
            event_user_id,
            ingest_source_id: c
                .event
                .as_ref()
                .and_then(|event| event.ingest.as_ref())
                .and_then(|ingest| ingest.source_id)
                .map(|id| id.to_string())
                .unwrap_or_default(),
            ingest_source_kind: c
                .event
                .as_ref()
                .and_then(|event| event.ingest.as_ref())
                .and_then(|ingest| ingest.source_kind)
                .map(|kind| kind.as_str().into()),
            ingest_reason: c
                .event
                .as_ref()
                .and_then(|event| event.ingest.as_ref())
                .and_then(|ingest| ingest.reason_code.clone())
                .unwrap_or_default(),
            ingest_retryable: c
                .event
                .as_ref()
                .and_then(|event| event.ingest.as_ref())
                .and_then(|ingest| ingest.retryable)
                .map(|value| value.to_string())
                .unwrap_or_default(),
            metadata_field_id: metadata
                .and_then(|value| value.field_id)
                .map(|id| id.to_string())
                .unwrap_or_default(),
            metadata_kind: metadata
                .and_then(|value| value.kind)
                .or_else(|| before.or(after).map(MetadataValue::kind))
                .map(|kind| kind.as_str().into())
                .unwrap_or_default(),
            metadata_state: metadata
                .and_then(|value| value.state)
                .map(|state| state.as_str().into())
                .unwrap_or_default(),
            metadata_before_enabled: before.map(|_| "on".into()),
            metadata_before,
            metadata_before_currency,
            metadata_after_enabled: after.map(|_| "on".into()),
            metadata_after,
            metadata_after_currency,
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
        };
        match c
            .event
            .as_ref()
            .and_then(|event| event.document_change.as_ref())
        {
            Some(WorkflowDocumentConditions::Title { before, after }) => {
                values.title_before_enabled = before.as_ref().map(|_| "on".into());
                values.title_before = before.clone().unwrap_or_default();
                values.title_after_enabled = after.as_ref().map(|_| "on".into());
                values.title_after = after.clone().unwrap_or_default();
            }
            Some(WorkflowDocumentConditions::Tags {
                added_id,
                removed_id,
                state,
            }) => {
                values.added_tag_id = condition_value(*added_id);
                values.removed_tag_id = condition_value(*removed_id);
                values.tag_change_state = condition_value(*state);
            }
            Some(
                WorkflowDocumentConditions::Correspondent {
                    before_id,
                    after_id,
                    state,
                }
                | WorkflowDocumentConditions::DocumentType {
                    before_id,
                    after_id,
                    state,
                },
            ) => {
                values.assignment_before_id = condition_value(*before_id);
                values.assignment_after_id = condition_value(*after_id);
                values.assignment_change_state = condition_value(*state);
            }
            Some(WorkflowDocumentConditions::Date {
                before,
                after,
                state,
            }) => {
                values.document_date_before = condition_value(*before);
                values.document_date_after = condition_value(*after);
                values.document_date_state = condition_value(*state);
            }
            Some(WorkflowDocumentConditions::Review { origin, reason }) => {
                values.review_origin = condition_value(*origin);
                values.unreview_reason = condition_value(*reason);
            }
            Some(WorkflowDocumentConditions::Revision { number, source }) => {
                values.revision_number =
                    number.map(|number| number.to_string()).unwrap_or_default();
                values.revision_source = condition_value(*source);
            }
            None => {}
        }
        values
    }

    fn cadence(&self, trigger: WorkflowTrigger) -> Result<Option<WorkflowCadence>, String> {
        if trigger != WorkflowTrigger::Scheduled {
            return Ok(None);
        }
        let start = self
            .cadence_start
            .trim()
            .parse()
            .map_err(|_| "Enter a UTC date and time such as 2026-12-01T09:00:00Z.")?;
        let cadence = match self.cadence_mode.as_str() {
            "once" => WorkflowCadence::Once { at: start },
            "recurring" => WorkflowCadence::Recurring {
                first_at: start,
                interval_seconds: self
                    .cadence_interval
                    .trim()
                    .parse()
                    .map_err(|_| "Enter a positive interval in whole seconds.")?,
            },
            _ => return Err("Choose a schedule frequency.".into()),
        };
        cadence.validate().map_err(|err| err.to_string())?;
        Ok(Some(cadence))
    }

    /// Turn the submission into a workflow, keeping any actions the form cannot edit
    fn into_input(self, existing: Option<&Workflow>) -> Result<WorkflowInput, String> {
        if self.ordered_actions.is_some()
            && existing.is_some_and(|workflow| self.revision != Some(workflow.revision))
        {
            return Err(
                "The workflow changed. Review the saved settings before trying again.".into(),
            );
        }
        let trigger = WorkflowTrigger::parse(&self.trigger).ok_or("Choose a trigger.")?;
        let triggers = self.explicit_triggers(trigger)?;
        let selected = if triggers.is_empty() {
            std::slice::from_ref(&trigger)
        } else {
            &triggers
        };
        let metadata_trigger = selected
            .iter()
            .copied()
            .find(|trigger| trigger.is_custom_metadata())
            .unwrap_or(trigger);
        let cadence = self.cadence(trigger)?;
        let optional = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
        let id = |s: &str, what: &str| -> Result<Option<Id>, String> {
            optional(s)
                .map(|v| Uuid::parse_str(&v).map_err(|_| format!("The {what} is not valid.")))
                .transpose()
        };
        let clear_clock_filters = cadence.is_some() && self.clear_clock_filters.is_some();
        let processing = if clear_clock_filters {
            None
        } else {
            let processing_trigger = selected
                .iter()
                .copied()
                .find(|trigger| trigger.is_processing_outcome());
            processing_condition(&self, existing, processing_trigger.unwrap_or(trigger))?
        };
        let mut event = match self.event_actor.as_deref() {
            _ if clear_clock_filters => None,
            None if trigger == WorkflowTrigger::IngestReceived => None,
            None => existing.and_then(|workflow| workflow.conditions.event.clone()),
            Some(mode) => {
                let subject_id = id(&self.event_subject_id, "event subject ID")?;
                let actor = match mode {
                    "" => None,
                    "system" => Some(WorkflowActorCondition::System),
                    "user" => Some(WorkflowActorCondition::User {
                        user_id: UserId::from(
                            optional(&self.event_user_id)
                                .as_deref()
                                .ok_or("Enter a user ID.")?,
                        ),
                    }),
                    _ => return Err("Choose an event actor.".into()),
                };
                let previous_ingest = existing
                    .and_then(|workflow| workflow.conditions.event.as_ref())
                    .and_then(|event| event.ingest.as_ref());
                let ingest = if !selected.iter().any(|trigger| trigger.is_ingest_outcome()) {
                    None
                } else if let Some(kind) = self.ingest_source_kind.as_deref() {
                    let source_kind = match kind {
                        "" => None,
                        "folder" => Some(SourceKind::Folder),
                        "email" => Some(SourceKind::Email),
                        _ => return Err("Choose an ingestion source kind.".into()),
                    };
                    let retryable = match self.ingest_retryable.as_str() {
                        "" => None,
                        "true" => Some(true),
                        "false" => Some(false),
                        _ => return Err("Choose a retry setting.".into()),
                    };
                    let filters = WorkflowIngestConditions {
                        source_id: id(&self.ingest_source_id, "source ID")?,
                        source_kind,
                        outcome: previous_ingest.and_then(|ingest| ingest.outcome),
                        reason_code: optional(&self.ingest_reason),
                        retryable,
                    };
                    (filters != WorkflowIngestConditions::default()).then_some(filters)
                } else {
                    previous_ingest.cloned()
                };
                let metadata = metadata_condition(&self, metadata_trigger)?;
                let document_change = self.document_condition(trigger)?;
                (subject_id.is_some()
                    || actor.is_some()
                    || ingest.is_some()
                    || metadata.is_some()
                    || document_change.is_some())
                .then_some(WorkflowEventConditions {
                    subject_id,
                    actor,
                    ingest,
                    metadata,
                    document_change,
                    processing: None,
                })
            }
        };
        if processing.is_some() || self.extra_fields.contains_key("processing_form") {
            if processing.is_some() || event.is_some() {
                event
                    .get_or_insert_with(WorkflowEventConditions::default)
                    .processing = processing;
            }
            if event == Some(WorkflowEventConditions::default()) {
                event = None;
            }
        }
        let mut conditions = existing
            .map(|workflow| workflow.conditions.clone())
            .unwrap_or_default();
        if clear_clock_filters {
            conditions = WorkflowConditions {
                all: conditions.all,
                any: conditions.any,
                none: conditions.none,
                ..WorkflowConditions::default()
            };
        } else if cadence.is_none() {
            conditions.event = event;
        }
        if let Some(groups) = &self.condition_groups {
            serde_json::from_str::<ConditionGroups>(groups)
                .map_err(|_| "Check the values in the nested condition groups.")?
                .apply(&mut conditions);
        }
        if cadence.is_none()
            && (selected.iter().any(|trigger| {
                trigger.entry_transition().is_none()
                    && !trigger.is_ingest_outcome()
                    && !trigger.is_custom_metadata()
                    && !trigger.is_saved_view()
            }) || (self.trigger_selection.is_some()
                && selected.iter().any(|trigger| trigger.is_saved_view())))
        {
            conditions.mime_type = optional(&self.mime_type);
            conditions.tag_id = id(&self.tag_id, "tag")?;
            conditions.correspondent_id = id(&self.correspondent_id, "correspondent")?;
            conditions.document_type_id = id(&self.document_type_id, "document type")?;
            conditions.filename_glob = optional(&self.filename_glob);
            conditions.source = SourceConditions {
                kind: optional(&self.source_kind).map(|k| SourceKind::parse(&k)),
                folder_path_glob: optional(&self.folder_path_glob),
                email_from: text_match(&self.email_from_mode, &self.email_from),
                email_to: text_match(&self.email_to_mode, &self.email_to),
                email_subject: text_match(&self.email_subject_mode, &self.email_subject),
                email_attachment_glob: optional(&self.email_attachment_glob),
                email_mailbox_glob: optional(&self.email_mailbox_glob),
            };
        } else if cadence.is_none()
            && self.trigger_selection.is_none()
            && existing.is_some_and(|workflow| {
                workflow.trigger.entry_transition().is_none()
                    && !workflow.trigger.is_ingest_outcome()
                    && !workflow.trigger.is_custom_metadata()
                    && !workflow.trigger.is_saved_view()
            })
        {
            conditions.mime_type = None;
            conditions.tag_id = None;
            conditions.correspondent_id = None;
            conditions.document_type_id = None;
            conditions.filename_glob = None;
            conditions.source = SourceConditions::default();
        }

        if let Some(plan) = &self.ordered_actions {
            let actions = resolve_actions(plan, existing)?;
            let actions = if trigger == WorkflowTrigger::IngestReceived && actions.is_empty() {
                vec![WorkflowAction::SkipImport]
            } else {
                actions
            };
            let input = WorkflowInput {
                name: self.name.trim().to_owned(),
                enabled: self.enabled.is_some(),
                trigger,
                triggers,
                cadence,
                conditions,
                actions,
            };
            input.validate().map_err(|error| match error {
                Error::Validation(message) => message,
                other => other.to_string(),
            })?;
            return Ok(input);
        }

        if trigger == WorkflowTrigger::IngestReceived {
            return Ok(WorkflowInput {
                cadence: None,
                name: self.name.trim().to_string(),
                enabled: self.enabled.is_some(),
                trigger,
                triggers,
                conditions,
                actions: vec![WorkflowAction::SkipImport],
            });
        }
        let mut headers = parse_headers(&self.webhook_headers)?;
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
        if cadence.is_some() && self.clear_clock_actions.is_some() {
            actions.retain(WorkflowAction::is_webhook);
        } else if cadence.is_none() {
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
        }
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
            cadence,
            name: self.name.trim().to_string(),
            enabled: self.enabled.is_some(),
            trigger,
            triggers,
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
        (None, Some(action)) => {
            let index = actions
                .iter()
                .position(|existing| matches!(existing, WorkflowAction::TrashDocument))
                .unwrap_or(actions.len());
            actions.insert(index, action);
        }
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

fn trigger_choices(selected: &str) -> Vec<TriggerChoice> {
    WorkflowTriggerRegistry::built_in()
        .definitions()
        .iter()
        .map(|trigger| TriggerChoice {
            value: trigger.event_type.clone(),
            label: trigger.label.clone(),
            selected: trigger.event_type == selected,
            document_actions: trigger.target == WorkflowTarget::Document,
            processing_keys: if trigger.event_type.starts_with("document.processing.") {
                trigger
                    .condition_fields
                    .iter()
                    .map(processing_key)
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                String::new()
            },
        })
        .collect()
}

struct ProcessingControl {
    key: String,
    label: String,
    value: String,
    boolean: bool,
    integer: bool,
    timestamp: bool,
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
    triggers: Vec<TriggerChoice>,
    additional_triggers: Vec<AdditionalTrigger>,
    processing_fields: Vec<ProcessingControl>,
    action_fields: String,
    metadata_action_catalog: String,
    reenter_credentials: bool,
    has_saved_root_filters: bool,
    has_saved_non_webhook_actions: bool,
    condition_groups: String,
    metadata_fields: Vec<Choice>,
    metadata_kinds: Vec<Choice>,
    metadata_states: Vec<Choice>,
    tags: Vec<Choice>,
    correspondents: Vec<Choice>,
    document_types: Vec<Choice>,
    source_kinds: Vec<Choice>,
    added_tags: Vec<Choice>,
    removed_tags: Vec<Choice>,
    previous_correspondents: Vec<Choice>,
    new_correspondents: Vec<Choice>,
    previous_document_types: Vec<Choice>,
    new_document_types: Vec<Choice>,
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
    mut values: FormValues,
    notice: Option<Notice>,
) -> ApiResult<FormTemplate> {
    if values.ordered_actions.is_none() {
        values.revision = workflow.map(|workflow| workflow.revision);
    }
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
    let additional =
        if values.trigger_selection.is_none() && values.additional_triggers().is_empty() {
            workflow
                .filter(|workflow| workflow.trigger.as_str() == values.trigger)
                .map(|workflow| {
                    workflow
                        .triggers
                        .iter()
                        .enumerate()
                        .skip(1)
                        .map(|(position, trigger)| (position, trigger.as_str()))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            values.additional_triggers()
        };
    let additional_triggers = additional
        .into_iter()
        .map(|(position, value)| AdditionalTrigger {
            position,
            choices: trigger_choices(value),
        })
        .collect();
    let delta_choices = |items, current: &str| {
        let mut options = choices(items, current);
        if !current.is_empty() && !options.iter().any(|option| option.selected) {
            options.push(Choice {
                value: current.into(),
                label: format!("Unavailable choice ({current})"),
                selected: true,
            });
        }
        options
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
    let mut metadata_fields = state.metadata_fields.list(vault, false, None, 100).await?;
    if let Ok(selected) = Uuid::parse_str(&values.metadata_field_id) {
        if !metadata_fields.iter().any(|field| field.id == selected) {
            if let Some(field) = state.metadata_fields.find(vault, selected).await? {
                metadata_fields.push(field);
            }
        }
    }
    metadata_fields.sort_by(|a, b| a.name.cmp(&b.name));
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

    let condition_groups = match values.condition_groups.as_deref() {
        Some(draft) if serde_json::from_str::<serde_json::Value>(draft).is_ok() => draft.to_owned(),
        _ => serde_json::to_string(
            &workflow
                .map(|w| ConditionGroups::from_conditions(&w.conditions))
                .unwrap_or_default(),
        )
        .map_err(|error| ApiError::internal(error.to_string()))?,
    };

    let (action_fields, reenter_credentials) =
        action_fields(values.ordered_actions.as_deref(), workflow)?;
    let has_saved_root_filters = workflow.is_some_and(|workflow| {
        let mut root = workflow.conditions.clone();
        ConditionGroups::default().apply(&mut root);
        root != WorkflowConditions::default()
    });
    let has_saved_non_webhook_actions =
        workflow.is_some_and(|workflow| workflow.actions.iter().any(|action| !action.is_webhook()));
    Ok(FormTemplate {
        condition_groups,
        action_fields,
        metadata_action_catalog: format!("/api/vaults/{vault}/metadata-fields"),
        reenter_credentials,
        has_saved_root_filters,
        has_saved_non_webhook_actions,
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
        triggers: trigger_choices(&values.trigger),
        additional_triggers,
        processing_fields: processing_fields()
            .map(|field| {
                let key = processing_key(field);
                ProcessingControl {
                    key: key.into(),
                    label: field.label.clone(),
                    value: values
                        .extra_fields
                        .get(&format!("processing_{key}"))
                        .cloned()
                        .unwrap_or_default(),
                    boolean: field.value_type == WorkflowConditionType::Boolean,
                    integer: field.value_type == WorkflowConditionType::Integer,
                    timestamp: field.value_type == WorkflowConditionType::Timestamp,
                }
            })
            .collect(),
        metadata_fields: choices(
            metadata_fields
                .into_iter()
                .map(|field| {
                    let label = if field.deleted_at.is_some() {
                        format!("{} (retired)", field.name)
                    } else {
                        field.name
                    };
                    (field.id.to_string(), label)
                })
                .collect(),
            &values.metadata_field_id,
        ),
        metadata_kinds: choices(
            METADATA_KINDS
                .into_iter()
                .map(|(kind, label)| (kind.as_str().into(), label.into()))
                .collect(),
            &values.metadata_kind,
        ),
        metadata_states: choices(
            [
                ("set", "Set for the first time"),
                ("replaced", "Value replaced"),
                ("cleared", "Value cleared"),
            ]
            .into_iter()
            .map(|(state, label)| (state.into(), label.into()))
            .collect(),
            &values.metadata_state,
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
        added_tags: delta_choices(
            pairs(tags.iter().map(|tag| (tag.id, tag.name.clone())).collect()),
            &values.added_tag_id,
        ),
        removed_tags: delta_choices(
            pairs(tags.iter().map(|tag| (tag.id, tag.name.clone())).collect()),
            &values.removed_tag_id,
        ),
        previous_correspondents: delta_choices(
            pairs(
                correspondents
                    .iter()
                    .map(|item| (item.id, item.name.clone()))
                    .collect(),
            ),
            &values.assignment_before_id,
        ),
        new_correspondents: delta_choices(
            pairs(
                correspondents
                    .iter()
                    .map(|item| (item.id, item.name.clone()))
                    .collect(),
            ),
            &values.assignment_after_id,
        ),
        previous_document_types: delta_choices(
            pairs(
                document_types
                    .iter()
                    .map(|item| (item.id, item.name.clone()))
                    .collect(),
            ),
            &values.assignment_before_id,
        ),
        new_document_types: delta_choices(
            pairs(
                document_types
                    .iter()
                    .map(|item| (item.id, item.name.clone()))
                    .collect(),
            ),
            &values.assignment_after_id,
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
    let saved = if values.ordered_actions.is_some() && values.revision != Some(workflow.revision) {
        Err(Error::Conflict(
            "The workflow changed. Review the saved settings before trying again.".into(),
        ))
    } else {
        match values.clone().into_input(Some(&workflow)) {
            Ok(input) => {
                state
                    .workflow_repo
                    .replace(vault, id, input, Some(workflow.revision))
                    .await
            }
            Err(message) => Err(Error::Validation(message)),
        }
    };
    match saved {
        Ok(_) => Ok(Redirect::to(&format!("/workflows/{id}")).into_response()),
        Err(Error::Conflict(message)) => {
            let latest = find(vault, &*state.workflow_repo, id).await?;
            let html = render(
                build_form(
                    DefaultVault(vault),
                    &state,
                    Some(&latest),
                    FormValues::from_workflow(&latest),
                    danger(message),
                )
                .await?,
            )?;
            Ok((StatusCode::CONFLICT, html).into_response())
        }
        Err(error) => Ok(render(
            build_form(
                DefaultVault(vault),
                &state,
                Some(&workflow),
                values,
                danger(error.to_string()),
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
                cadence: workflow.cadence,
                name: workflow.name,
                enabled: !workflow.enabled,
                trigger: workflow.trigger,
                triggers: workflow.triggers,
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
