use serde::{Deserialize, Serialize};

use super::{Id, UserId};
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
pub struct WorkflowEventConditions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub subject_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<WorkflowActorCondition>,
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
            && match &self.actor {
                None => true,
                Some(WorkflowActorCondition::System) => event.actor_id().is_none(),
                Some(WorkflowActorCondition::User { user_id }) => event.actor_id() == Some(user_id),
            }
    }
}
