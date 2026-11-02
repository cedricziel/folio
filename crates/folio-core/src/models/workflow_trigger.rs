use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::WorkflowAction;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkflowTarget {
    Document,
    Event,
    IngestCandidate,
}

impl WorkflowTarget {
    /// Check target compatibility; this does not authorize an action or validate its fields.
    pub fn accepts(self, action: &WorkflowAction) -> bool {
        match action {
            WorkflowAction::Webhook { .. } => self != Self::IngestCandidate,
            WorkflowAction::SkipImport => self == Self::IngestCandidate,
            WorkflowAction::AddTags { .. }
            | WorkflowAction::SetCorrespondent { .. }
            | WorkflowAction::SetDocumentType { .. } => self == Self::Document,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkflowConditionType {
    Text,
    Boolean,
    Integer,
    Decimal,
    Date,
    Timestamp,
    Identifier,
    IdentifierSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowConditionField {
    pub key: String,
    pub label: String,
    pub value_type: WorkflowConditionType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowTriggerDefinition {
    pub event_type: String,
    pub label: String,
    pub subject_type: String,
    pub target: WorkflowTarget,
    pub condition_fields: Vec<WorkflowConditionField>,
}

impl WorkflowTriggerDefinition {
    fn validate(&self) -> Result<()> {
        let valid_part = |part: &str| {
            !part.is_empty()
                && part.len() <= 64
                && part.as_bytes()[0].is_ascii_lowercase()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        };
        let mut parts = self.event_type.split('.');
        if parts.next() != Some(self.subject_type.as_str())
            || !valid_part(&self.subject_type)
            || !parts.next().is_some_and(valid_part)
            || !parts.all(valid_part)
            || self.label.trim().is_empty()
        {
            return Err(Error::Validation(
                "Invalid workflow trigger definition".into(),
            ));
        }
        let mut keys = BTreeSet::new();
        for field in &self.condition_fields {
            if !field.key.split('.').all(valid_part)
                || field.label.trim().is_empty()
                || !keys.insert(&field.key)
            {
                return Err(Error::Validation(
                    "Invalid workflow condition field catalog".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowTriggerRegistry {
    definitions: Vec<WorkflowTriggerDefinition>,
}

impl WorkflowTriggerRegistry {
    /// Register a producer catalog, rejecting malformed or duplicate definitions.
    pub fn new(definitions: Vec<WorkflowTriggerDefinition>) -> Result<Self> {
        let mut registry = Self::default();
        for definition in definitions {
            registry.register(definition)?;
        }
        Ok(registry)
    }

    /// Add an event without replacing an existing producer's definition.
    pub fn register(&mut self, definition: WorkflowTriggerDefinition) -> Result<()> {
        definition.validate()?;
        if self.get(&definition.event_type).is_some() {
            return Err(Error::Validation(format!(
                "Workflow trigger '{}' is already registered",
                definition.event_type
            )));
        }
        self.definitions.push(definition);
        Ok(())
    }

    pub fn definitions(&self) -> &[WorkflowTriggerDefinition] {
        &self.definitions
    }

    pub fn get(&self, event_type: &str) -> Option<&WorkflowTriggerDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.event_type == event_type)
    }

    /// Every selected trigger must support every action. Does not check access or action fields.
    pub fn validate_actions(&self, selected: &[&str], actions: &[WorkflowAction]) -> Result<()> {
        if selected.is_empty() || actions.is_empty() {
            return Err(Error::Validation(
                "A workflow needs at least one trigger and action".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for event_type in selected {
            if !seen.insert(event_type) {
                return Err(Error::Validation(
                    "A workflow cannot select the same trigger twice".into(),
                ));
            }
            let definition = self.get(event_type).ok_or_else(|| {
                Error::Validation(format!("Unknown workflow trigger '{event_type}'"))
            })?;
            if actions
                .iter()
                .any(|action| !definition.target.accepts(action))
            {
                return Err(Error::Validation(format!(
                    "An action is incompatible with workflow trigger '{event_type}'"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Id, WorkflowAction};

    fn definition(event_type: &str, target: WorkflowTarget) -> WorkflowTriggerDefinition {
        WorkflowTriggerDefinition {
            event_type: event_type.into(),
            label: "When something changes".into(),
            subject_type: event_type.split('.').next().unwrap().into(),
            target,
            condition_fields: vec![],
        }
    }

    fn webhook() -> WorkflowAction {
        WorkflowAction::Webhook {
            url: "https://example.com/hook".into(),
            method: Default::default(),
            headers: Default::default(),
            secret: None,
        }
    }

    #[test]
    fn duplicate_registration_does_not_replace_the_original() {
        let original = definition("tag.created", WorkflowTarget::Event);
        let mut registry = WorkflowTriggerRegistry::new(vec![original.clone()]).unwrap();
        assert!(registry
            .register(definition("tag.created", WorkflowTarget::Document))
            .is_err());
        assert_eq!(registry.get("tag.created"), Some(&original));
    }

    #[test]
    fn unknown_event_names_are_not_implicitly_enabled() {
        let registry =
            WorkflowTriggerRegistry::new(vec![definition("tag.created", WorkflowTarget::Event)])
                .unwrap();
        assert!(registry
            .validate_actions(&["tag.deleted"], &[webhook()])
            .is_err());
    }

    #[test]
    fn every_selected_trigger_must_accept_every_action() {
        let registry = WorkflowTriggerRegistry::new(vec![
            definition("document.updated", WorkflowTarget::Document),
            definition("tag.created", WorkflowTarget::Event),
        ])
        .unwrap();
        let action = WorkflowAction::AddTags {
            tag_ids: vec![Id::new_v4()],
        };
        assert!(registry
            .validate_actions(&["document.updated"], std::slice::from_ref(&action))
            .is_ok());
        assert!(registry
            .validate_actions(&["document.updated", "tag.created"], &[action])
            .is_err());
        assert!(registry
            .validate_actions(&["document.updated", "tag.created"], &[webhook()])
            .is_ok());
    }

    #[test]
    fn skip_import_is_exclusive_to_ingestion_candidates() {
        let registry = WorkflowTriggerRegistry::new(vec![
            definition("ingest.received", WorkflowTarget::IngestCandidate),
            definition("ingestion_source.poll_completed", WorkflowTarget::Event),
        ])
        .unwrap();
        assert!(registry
            .validate_actions(&["ingest.received"], &[WorkflowAction::SkipImport])
            .is_ok());
        assert!(registry
            .validate_actions(
                &["ingestion_source.poll_completed"],
                &[WorkflowAction::SkipImport]
            )
            .is_err());
        assert!(registry
            .validate_actions(&["ingest.received"], &[webhook()])
            .is_err());
    }

    #[test]
    fn empty_or_repeated_trigger_selections_are_rejected() {
        let registry =
            WorkflowTriggerRegistry::new(vec![definition("tag.created", WorkflowTarget::Event)])
                .unwrap();
        assert!(registry.validate_actions(&[], &[webhook()]).is_err());
        assert!(registry
            .validate_actions(&["tag.created", "tag.created"], &[webhook()])
            .is_err());
        assert!(registry.validate_actions(&["tag.created"], &[]).is_err());
    }

    #[test]
    fn registration_rejects_a_mismatched_subject() {
        let mut trigger = definition("document.updated", WorkflowTarget::Document);
        trigger.subject_type = "tag".into();
        assert!(WorkflowTriggerRegistry::new(vec![trigger]).is_err());
    }

    #[test]
    fn condition_catalog_keys_must_be_unique() {
        let mut trigger = definition("document.processing.completed", WorkflowTarget::Document);
        let field = WorkflowConditionField {
            key: "data.stage".into(),
            label: "Stage".into(),
            value_type: WorkflowConditionType::Text,
        };
        trigger.condition_fields = vec![field.clone(), field];
        assert!(WorkflowTriggerRegistry::new(vec![trigger]).is_err());
    }

    #[test]
    fn registered_catalog_preserves_labels_and_field_types() {
        let mut trigger = definition("document.tags_changed", WorkflowTarget::Document);
        trigger.label = "When tags change".into();
        trigger.condition_fields = vec![WorkflowConditionField {
            key: "data.added_ids".into(),
            label: "Added tags".into(),
            value_type: WorkflowConditionType::IdentifierSet,
        }];
        let registry = WorkflowTriggerRegistry::new(vec![trigger.clone()]).unwrap();
        assert_eq!(registry.definitions(), &[trigger]);
    }
}
