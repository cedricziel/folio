use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    Id, IngestResult, MetadataKind, MetadataValue, SourceKind, UserId, WebhookIngest,
    WebhookProcessing, WebhookWorkflowRun, WorkflowDocumentConditions, WorkflowTrigger,
    WorkflowTriggerRegistry,
};
use crate::processing_events::{ProcessingEventStage, ProcessingReason};
use crate::{DomainEventEnvelope, Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkflowActorCondition {
    System,
    User {
        #[cfg_attr(feature = "openapi", schema(value_type = String))]
        user_id: UserId,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowIngestConditions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub source_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<SourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<IngestResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

impl WorkflowIngestConditions {
    pub fn validate_trigger(&self, trigger: WorkflowTrigger) -> Result<()> {
        let invalid = !trigger.is_ingest_outcome()
            || (matches!(
                trigger,
                WorkflowTrigger::SourceFailed | WorkflowTrigger::SourceRecovered
            ) && (self.outcome.is_some() || self.retryable.is_some()))
            || (trigger == WorkflowTrigger::SourceRecovered && self.reason_code.is_some());
        if invalid {
            return Err(Error::Validation(
                "These ingestion conditions are unavailable for the selected trigger".into(),
            ));
        }
        if self
            .reason_code
            .as_ref()
            .is_some_and(|code| code.is_empty() || code.len() > 64)
        {
            return Err(Error::Validation("Invalid ingestion reason code".into()));
        }
        Ok(())
    }

    fn matches(&self, event: &DomainEventEnvelope) -> bool {
        let Ok(ingest) = WebhookIngest::deserialize(event.data()) else {
            return false;
        };
        self.source_id.is_none_or(|id| id == ingest.source_id)
            && self
                .source_kind
                .is_none_or(|kind| kind == ingest.source_kind)
            && self
                .outcome
                .is_none_or(|outcome| Some(outcome) == ingest.outcome)
            && self
                .reason_code
                .as_ref()
                .is_none_or(|code| Some(code) == ingest.reason_code.as_ref())
            && self
                .retryable
                .is_none_or(|retryable| Some(retryable) == ingest.retryable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum MetadataChangeState {
    Set,
    Replaced,
    Cleared,
}

impl MetadataChangeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Replaced => "replaced",
            Self::Cleared => "cleared",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowMetadataConditions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = MetadataKind, required = false))]
    pub kind: Option<MetadataKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = MetadataChangeState, required = false))]
    pub state: Option<MetadataChangeState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = MetadataValue, required = false))]
    pub before: Option<MetadataValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = MetadataValue, required = false))]
    pub after: Option<MetadataValue>,
}

impl WorkflowMetadataConditions {
    pub fn validate_trigger(&self, trigger: WorkflowTrigger) -> Result<()> {
        if !trigger.is_custom_metadata()
            || (trigger != WorkflowTrigger::DocumentMetadataChanged
                && (self.state.is_some() || self.before.is_some() || self.after.is_some()))
        {
            return Err(Error::Validation(
                "These custom field conditions are unavailable for the selected trigger".into(),
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        let kind = self
            .kind
            .or_else(|| self.before.as_ref().map(MetadataValue::kind));
        for value in [&self.before, &self.after].into_iter().flatten() {
            value
                .clone()
                .validate(kind.unwrap_or_else(|| value.kind()), false)?;
        }
        if (self.state == Some(MetadataChangeState::Set) && self.before.is_some())
            || (self.state == Some(MetadataChangeState::Cleared) && self.after.is_some())
        {
            return Err(Error::Validation(
                "Choose a change state that can have the requested value".into(),
            ));
        }
        Ok(())
    }

    fn matches(&self, event: &DomainEventEnvelope) -> bool {
        use crate::domain_event::DOCUMENT_METADATA_CHANGED;
        use serde_json::Value;

        let matches_field = |data: &Value| {
            self.field_id.is_none_or(|field| {
                data["field_id"]
                    .as_str()
                    .and_then(|id| uuid::Uuid::parse_str(id).ok())
                    == Some(field)
            })
        };
        let matches_kind =
            |kind: Option<&str>| self.kind.is_none_or(|wanted| Some(wanted.as_str()) == kind);
        let data = event.data();
        match event.event_type() {
            DOCUMENT_METADATA_CHANGED => {}
            name if name.starts_with("metadata_field.") || name.starts_with("metadata_option.") => {
                let kind = if name.starts_with("metadata_option.") {
                    Some(MetadataKind::Select.as_str())
                } else {
                    data["kind"].as_str()
                };
                return self.state.is_none()
                    && self.before.is_none()
                    && self.after.is_none()
                    && matches_field(data)
                    && matches_kind(kind);
            }
            _ => return false,
        }

        let normalize = |value: &Option<MetadataValue>| -> Result<Option<Value>> {
            value
                .as_ref()
                .map(|value| {
                    let normalized = value.clone().validate(value.kind(), false)?;
                    serde_json::to_value(normalized)
                        .map_err(|error| Error::Validation(error.to_string()))
                })
                .transpose()
        };
        let (Ok(before), Ok(after)) = (normalize(&self.before), normalize(&self.after)) else {
            return false;
        };
        data["changes"].as_array().is_some_and(|changes| {
            changes.iter().any(|change| {
                let (Some(from), Some(to)) = (change.get("before"), change.get("after")) else {
                    return false;
                };
                let state = match (from.is_null(), to.is_null()) {
                    (true, false) => MetadataChangeState::Set,
                    (false, false) => MetadataChangeState::Replaced,
                    (false, true) => MetadataChangeState::Cleared,
                    (true, true) => return false,
                };
                matches_field(change)
                    && matches_kind(change["kind"].as_str())
                    && self.state.is_none_or(|wanted| wanted == state)
                    && before.as_ref().is_none_or(|value| value == from)
                    && after.as_ref().is_none_or(|value| value == to)
            })
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowProcessingConditions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = ProcessingEventStage, required = false))]
    pub stage: Option<ProcessingEventStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = ProcessingReason, required = false))]
    pub reason_code: Option<ProcessingReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_text: Option<bool>,
}

impl WorkflowProcessingConditions {
    pub fn validate_trigger(&self, trigger: WorkflowTrigger) -> Result<()> {
        let definition = WorkflowTriggerRegistry::built_in().get(trigger.as_str());
        let supported = |key: &str| {
            trigger.is_processing_outcome()
                && definition.is_some_and(|definition| {
                    definition
                        .condition_fields
                        .iter()
                        .any(|field| field.key == key)
                })
        };
        if [
            (self.stage.is_some(), "data.stage"),
            (self.outcome.is_some(), "data.outcome"),
            (self.run_id.is_some(), "data.run_id"),
            (self.revision_id.is_some(), "data.revision_id"),
            (self.attempt.is_some(), "data.attempt"),
            (self.reason_code.is_some(), "data.reason_code"),
            (self.retry_at.is_some(), "data.retry_at"),
            (self.has_text.is_some(), "data.has_text"),
        ]
        .iter()
        .any(|(present, key)| *present && !supported(key))
        {
            return Err(Error::Validation(
                "These processing conditions are unavailable for the selected trigger".into(),
            ));
        }
        if !trigger.is_processing_outcome() {
            return Err(Error::Validation(
                "Processing conditions require a processing trigger".into(),
            ));
        }
        Ok(())
    }

    fn matches(&self, event: &DomainEventEnvelope) -> bool {
        let Ok(processing) = WebhookProcessing::deserialize(event.data()) else {
            return false;
        };
        self.stage.is_none_or(|value| processing.stage == value)
            && self
                .outcome
                .as_ref()
                .is_none_or(|value| processing.outcome == *value)
            && self.run_id.is_none_or(|value| processing.run_id == value)
            && self
                .revision_id
                .is_none_or(|value| processing.revision_id == Some(value))
            && self.attempt.is_none_or(|value| processing.attempt == value)
            && self
                .reason_code
                .is_none_or(|value| processing.reason_code == Some(value))
            && self
                .retry_at
                .is_none_or(|value| processing.retry_at == Some(value))
            && self
                .has_text
                .is_none_or(|value| processing.has_text == Some(value))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowEventConditions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub subject_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub workflow_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<WorkflowActorCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest: Option<WorkflowIngestConditions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WorkflowMetadataConditions, required = false))]
    pub metadata: Option<WorkflowMetadataConditions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WorkflowDocumentConditions, required = false))]
    pub document_change: Option<WorkflowDocumentConditions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WorkflowProcessingConditions, required = false))]
    pub processing: Option<WorkflowProcessingConditions>,
}

impl WorkflowEventConditions {
    pub fn validate(&self) -> Result<()> {
        if let Some(WorkflowActorCondition::User { user_id }) = &self.actor {
            if user_id.as_str().trim().is_empty() {
                return Err(Error::Validation("Event actor ID must not be empty".into()));
            }
        }
        if let Some(metadata) = &self.metadata {
            metadata.validate()?;
        }
        Ok(())
    }

    pub fn matches(&self, event: &DomainEventEnvelope) -> bool {
        self.subject_id.is_none_or(|id| event.subject().id == id)
            && self.workflow_id.is_none_or(|id| {
                WebhookWorkflowRun::from_envelope(event).is_some_and(|run| run.workflow_id == id)
            })
            && self
                .ingest
                .as_ref()
                .is_none_or(|condition| condition.matches(event))
            && self
                .metadata
                .as_ref()
                .is_none_or(|condition| condition.matches(event))
            && self
                .document_change
                .as_ref()
                .is_none_or(|condition| condition.matches(event))
            && self
                .processing
                .as_ref()
                .is_none_or(|condition| condition.matches(event))
            && match &self.actor {
                None => true,
                Some(WorkflowActorCondition::System) => event.actor_id().is_none(),
                Some(WorkflowActorCondition::User { user_id }) => event.actor_id() == Some(user_id),
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DomainEventSubject, EventAudience};
    use serde_json::json;

    #[test]
    fn metadata_conditions_compare_exact_values_from_the_same_change() {
        let document = uuid::Uuid::new_v4();
        let amount = uuid::Uuid::new_v4();
        let processed = uuid::Uuid::new_v4();
        let event = DomainEventEnvelope::new(
            "document.metadata.changed",
            super::super::VaultId(uuid::Uuid::new_v4()),
            DomainEventSubject {
                subject_type: "document".into(),
                id: document,
            },
            Some(document),
            EventAudience::Vault,
            json!({"changes": [
                {"field_id": amount, "kind": "decimal", "before": null,
                 "after": {"kind": "decimal", "value": "9007199254740993.01"}},
                {"field_id": processed, "kind": "boolean", "before": null,
                 "after": {"kind": "boolean", "value": false}}
            ]}),
        )
        .unwrap();
        let matches = |metadata| {
            let conditions: WorkflowEventConditions =
                serde_json::from_value(json!({"metadata": metadata})).unwrap();
            conditions.validate().unwrap();
            conditions.matches(&event)
        };
        assert!(matches(json!({"field_id": amount, "state": "set",
            "after": {"kind": "decimal", "value": "9007199254740993.0100"}})));
        assert!(!matches(json!({"field_id": amount,
            "after": {"kind": "decimal", "value": "9007199254740993.02"}})));
        assert!(!matches(
            json!({"field_id": amount, "after": {"kind": "boolean", "value": false}})
        ));
        assert!(matches(
            json!({"field_id": processed, "after": {"kind": "boolean", "value": false}})
        ));
        assert!(!matches(json!({"field_id": processed, "state": "cleared"})));
    }
}
