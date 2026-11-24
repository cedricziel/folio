use std::collections::{BTreeMap, BTreeSet};

use folio_core::models::{Id, WebhookMethod, Workflow, WorkflowAction};
use serde::{Deserialize, Serialize};

use super::workflows::{restore_webhook_secrets, SECRET_MASK};
use crate::error::ApiError;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ActionKind {
    Webhook,
    AddTags,
    SetCorrespondent,
    SetDocumentType,
    RenameDocument,
    RemoveTags,
    ClearCorrespondent,
    ClearDocumentType,
    TrashDocument,
    SetDocumentDate,
    ClearDocumentDate,
    SetReviewed,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ActionRow {
    kind: ActionKind,
    existing: Option<usize>,
    #[serde(default)]
    value: String,
    #[serde(default)]
    tag_ids: Vec<Id>,
    #[serde(default)]
    headers: String,
    #[serde(default)]
    secret: String,
    #[serde(default)]
    remove_secret: bool,
    #[serde(default)]
    has_secret: bool,
}

impl ActionRow {
    fn new(kind: ActionKind, index: usize) -> Self {
        Self {
            kind,
            existing: Some(index),
            value: String::new(),
            tag_ids: Vec::new(),
            headers: String::new(),
            secret: String::new(),
            remove_secret: false,
            has_secret: false,
        }
    }

    fn from_saved(index: usize, action: &WorkflowAction) -> Option<Self> {
        Some(match action {
            WorkflowAction::Webhook {
                url,
                headers,
                secret,
                ..
            } => Self {
                value: url.to_owned(),
                headers: headers
                    .iter()
                    .map(|(name, value)| {
                        let shown = if value.is_empty() { "" } else { SECRET_MASK };
                        format!("{name}: {shown}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                has_secret: secret.as_ref().is_some_and(|value| !value.is_empty()),
                ..Self::new(ActionKind::Webhook, index)
            },
            WorkflowAction::AddTags { tag_ids } => Self {
                tag_ids: tag_ids.clone(),
                ..Self::new(ActionKind::AddTags, index)
            },
            WorkflowAction::SetCorrespondent { correspondent_id } => Self {
                value: correspondent_id.to_string(),
                ..Self::new(ActionKind::SetCorrespondent, index)
            },
            WorkflowAction::SetDocumentType { document_type_id } => Self {
                value: document_type_id.to_string(),
                ..Self::new(ActionKind::SetDocumentType, index)
            },
            WorkflowAction::RenameDocument { title } => Self {
                value: title.clone(),
                ..Self::new(ActionKind::RenameDocument, index)
            },
            WorkflowAction::RemoveTags { tag_ids } => Self {
                tag_ids: tag_ids.clone(),
                ..Self::new(ActionKind::RemoveTags, index)
            },
            WorkflowAction::ClearCorrespondent => Self::new(ActionKind::ClearCorrespondent, index),
            WorkflowAction::ClearDocumentType => Self::new(ActionKind::ClearDocumentType, index),
            WorkflowAction::TrashDocument => Self::new(ActionKind::TrashDocument, index),
            WorkflowAction::SetDocumentDate { date } => Self {
                value: date.to_string(),
                ..Self::new(ActionKind::SetDocumentDate, index)
            },
            WorkflowAction::ClearDocumentDate => Self::new(ActionKind::ClearDocumentDate, index),
            WorkflowAction::SetReviewed { reviewed } => Self {
                value: reviewed.to_string(),
                ..Self::new(ActionKind::SetReviewed, index)
            },
            WorkflowAction::SkipImport => return None,
        })
    }

    fn into_action(self, saved: Option<&WorkflowAction>) -> Result<WorkflowAction, String> {
        let id = |name| self.value.parse().map_err(|_| format!("Choose a {name}."));
        let mut action = match self.kind {
            ActionKind::AddTags => WorkflowAction::AddTags {
                tag_ids: self.tag_ids,
            },
            ActionKind::SetCorrespondent => WorkflowAction::SetCorrespondent {
                correspondent_id: id("correspondent")?,
            },
            ActionKind::SetDocumentType => WorkflowAction::SetDocumentType {
                document_type_id: id("document type")?,
            },
            ActionKind::RenameDocument => WorkflowAction::RenameDocument { title: self.value },
            ActionKind::RemoveTags => WorkflowAction::RemoveTags {
                tag_ids: self.tag_ids,
            },
            ActionKind::ClearCorrespondent => WorkflowAction::ClearCorrespondent,
            ActionKind::ClearDocumentType => WorkflowAction::ClearDocumentType,
            ActionKind::TrashDocument => WorkflowAction::TrashDocument,
            ActionKind::SetDocumentDate => WorkflowAction::SetDocumentDate {
                date: self.value.parse().map_err(|_| "Choose a document date.")?,
            },
            ActionKind::ClearDocumentDate => WorkflowAction::ClearDocumentDate,
            ActionKind::SetReviewed => WorkflowAction::SetReviewed {
                reviewed: self.value.parse().map_err(|_| "Choose a review state.")?,
            },
            ActionKind::Webhook => {
                let url = self.value.trim().to_owned();
                let headers = parse_headers(&self.headers)?;
                let secret = if self.remove_secret {
                    None
                } else if !self.secret.is_empty() {
                    Some(self.secret)
                } else {
                    match saved {
                        Some(WorkflowAction::Webhook {
                            url: stored_url,
                            secret: Some(_),
                            ..
                        }) if stored_url == &url => Some(SECRET_MASK.to_owned()),
                        _ => None,
                    }
                };
                WorkflowAction::Webhook {
                    url,
                    method: WebhookMethod::Post,
                    headers,
                    secret,
                }
            }
        };
        restore_webhook_secrets(&mut action, saved).map_err(|error| error.message().to_owned())?;
        Ok(action)
    }
}

pub(super) fn parse_headers(text: &str) -> Result<BTreeMap<String, String>, String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (name, value) = line
                .split_once(':')
                .ok_or("Enter each header as Name: value.")?;
            Ok((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

pub(super) fn resolve_actions(
    plan: &str,
    saved: Option<&Workflow>,
) -> Result<Vec<WorkflowAction>, String> {
    let rows: Vec<ActionRow> =
        serde_json::from_str(plan).map_err(|_| "The action list is invalid.")?;
    let mut seen = BTreeSet::new();
    rows.into_iter()
        .map(|row| {
            let existing = match row.existing {
                Some(index) if seen.insert(index) => Some(
                    saved
                        .and_then(|workflow| workflow.actions.get(index))
                        .ok_or("A saved action is unavailable. Reload the workflow.")?,
                ),
                Some(_) => return Err("A saved action can only appear once.".into()),
                None => None,
            };
            row.into_action(existing)
        })
        .collect()
}

pub(super) fn action_fields(
    submitted: Option<&str>,
    saved: Option<&Workflow>,
) -> Result<(String, bool), ApiError> {
    let mut rows = submitted
        .and_then(|plan| serde_json::from_str::<Vec<ActionRow>>(plan).ok())
        .unwrap_or_else(|| {
            saved
                .into_iter()
                .flat_map(|workflow| workflow.actions.iter().enumerate())
                .filter_map(|(index, action)| ActionRow::from_saved(index, action))
                .collect()
        });
    let mut reenter_credentials = false;
    for row in &mut rows {
        if !row.secret.is_empty() {
            row.secret.clear();
            reenter_credentials = true;
        }
        row.headers = row
            .headers
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter(|line| {
                let keep = line
                    .split_once(':')
                    .is_some_and(|(_, value)| matches!(value.trim(), "" | SECRET_MASK));
                reenter_credentials |= !keep;
                keep
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    let json =
        serde_json::to_string(&rows).map_err(|error| ApiError::internal(error.to_string()))?;
    Ok((json, reenter_credentials))
}
