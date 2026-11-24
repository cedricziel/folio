use folio_core::models::{Id, UpdateDocument, WorkflowAction};
use folio_core::{Error, Result};

pub(super) enum DocumentAction<'a> {
    Update(UpdateDocument),
    Tags { ids: &'a [Id], add: bool },
    Trash,
    Review(bool),
}

impl<'a> TryFrom<&'a WorkflowAction> for DocumentAction<'a> {
    type Error = Error;

    fn try_from(action: &'a WorkflowAction) -> Result<Self> {
        Ok(match action {
            WorkflowAction::RenameDocument { title } => {
                WorkflowAction::validate_document_title(title)?;
                Self::Update(UpdateDocument {
                    title: Some(title.clone()),
                    ..Default::default()
                })
            }
            WorkflowAction::AddTags { tag_ids } => Self::Tags {
                ids: tag_ids,
                add: true,
            },
            WorkflowAction::RemoveTags { tag_ids } => Self::Tags {
                ids: tag_ids,
                add: false,
            },
            WorkflowAction::SetCorrespondent { correspondent_id } => Self::Update(UpdateDocument {
                correspondent_id: Some(Some(*correspondent_id)),
                ..Default::default()
            }),
            WorkflowAction::SetDocumentType { document_type_id } => Self::Update(UpdateDocument {
                document_type_id: Some(Some(*document_type_id)),
                ..Default::default()
            }),
            WorkflowAction::ClearCorrespondent => Self::Update(UpdateDocument {
                correspondent_id: Some(None),
                ..Default::default()
            }),
            WorkflowAction::ClearDocumentType => Self::Update(UpdateDocument {
                document_type_id: Some(None),
                ..Default::default()
            }),
            WorkflowAction::TrashDocument => Self::Trash,
            WorkflowAction::SetDocumentDate { date } => Self::Update(UpdateDocument {
                document_date: Some(Some(*date)),
                ..Default::default()
            }),
            WorkflowAction::ClearDocumentDate => Self::Update(UpdateDocument {
                document_date: Some(None),
                ..Default::default()
            }),
            WorkflowAction::SetReviewed { reviewed } => Self::Review(*reviewed),
            WorkflowAction::Webhook { .. } | WorkflowAction::SkipImport => {
                return Err(Error::Validation(
                    "Expected a document workflow action".into(),
                ));
            }
        })
    }
}

pub(super) fn run_stopped() -> Error {
    Error::Conflict("Workflow action is no longer current".into())
}

pub(super) fn pending_action(
    completed: i64,
    index: u32,
    actions: &str,
) -> Result<Option<(WorkflowAction, bool)>> {
    match completed.cmp(&i64::from(index)) {
        std::cmp::Ordering::Greater => Ok(None),
        std::cmp::Ordering::Less => Err(run_stopped()),
        std::cmp::Ordering::Equal => {
            let actions: Vec<WorkflowAction> = serde_json::from_str(actions)
                .map_err(|error| Error::Repository(format!("Invalid workflow actions: {error}")))?;
            let is_final = actions.len() == index as usize + 1;
            let action = actions
                .into_iter()
                .nth(index as usize)
                .ok_or_else(run_stopped)?;
            Ok(Some((action, is_final)))
        }
    }
}
