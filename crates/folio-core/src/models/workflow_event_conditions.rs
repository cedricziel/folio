use serde::{Deserialize, Serialize};

use super::{Id, IngestResult, SourceKind, UserId, WebhookIngest, WorkflowTrigger};
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowEventConditions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub subject_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<WorkflowActorCondition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest: Option<WorkflowIngestConditions>,
}

impl WorkflowEventConditions {
    pub fn validate(&self) -> Result<()> {
        if let Some(WorkflowActorCondition::User { user_id }) = &self.actor {
            if user_id.as_str().trim().is_empty() {
                return Err(Error::Validation("Event actor ID must not be empty".into()));
            }
        }
        Ok(())
    }

    pub fn matches(&self, event: &DomainEventEnvelope) -> bool {
        self.subject_id.is_none_or(|id| event.subject().id == id)
            && self
                .ingest
                .as_ref()
                .is_none_or(|condition| condition.matches(event))
            && match &self.actor {
                None => true,
                Some(WorkflowActorCondition::System) => event.actor_id().is_none(),
                Some(WorkflowActorCondition::User { user_id }) => event.actor_id() == Some(user_id),
            }
    }
}
