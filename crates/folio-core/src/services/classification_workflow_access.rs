use std::sync::Arc;

use crate::models::{Id, VaultId, WorkflowTrigger};
use crate::ports::{CorrespondentRepository, DocumentTypeRepository, TagRepository};
use crate::Result;

pub struct ClassificationWorkflowAccess {
    tags: Arc<dyn TagRepository>,
    correspondents: Arc<dyn CorrespondentRepository>,
    document_types: Arc<dyn DocumentTypeRepository>,
}

impl ClassificationWorkflowAccess {
    pub fn new(
        tags: Arc<dyn TagRepository>,
        correspondents: Arc<dyn CorrespondentRepository>,
        document_types: Arc<dyn DocumentTypeRepository>,
    ) -> Self {
        Self {
            tags,
            correspondents,
            document_types,
        }
    }

    pub async fn permits(&self, trigger: WorkflowTrigger, vault: VaultId, id: Id) -> Result<bool> {
        let deleted = trigger.as_str().ends_with(".deleted");
        Ok(match trigger.classification_subject() {
            Some("tag") => self
                .tags
                .find_retained(vault, id)
                .await?
                .is_some_and(|row| row.deleted_at.is_some() == deleted),
            Some("correspondent") => self
                .correspondents
                .find_retained(vault, id)
                .await?
                .is_some_and(|row| row.deleted_at.is_some() == deleted),
            Some("document_type") => self
                .document_types
                .find_retained(vault, id)
                .await?
                .is_some_and(|row| row.deleted_at.is_some() == deleted),
            _ => false,
        })
    }
}
