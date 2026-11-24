use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{DocumentDate, Id, MetadataChangeState, ReviewOrigin, WorkflowTrigger};
use crate::{DomainEventEnvelope, Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkflowUnreviewReason {
    User,
    System,
    FileReplaced,
    FileRestored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkflowRevisionSource {
    Upload,
    Replacement,
    Restoration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkflowDocumentConditions {
    Date {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = String, format = Date, required = false))]
        before: Option<DocumentDate>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = String, format = Date, required = false))]
        after: Option<DocumentDate>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = MetadataChangeState, required = false))]
        state: Option<MetadataChangeState>,
    },
    Review {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = ReviewOrigin, required = false))]
        origin: Option<ReviewOrigin>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = WorkflowUnreviewReason, required = false))]
        reason: Option<WorkflowUnreviewReason>,
    },
    Revision {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = i64, minimum = 1, required = false))]
        number: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = WorkflowRevisionSource, required = false))]
        source: Option<WorkflowRevisionSource>,
    },
}

impl WorkflowDocumentConditions {
    pub fn validate_trigger(&self, trigger: WorkflowTrigger) -> Result<()> {
        use WorkflowTrigger::*;
        let valid = match self {
            Self::Date {
                before,
                after,
                state,
            } => {
                trigger == DocumentDateChanged
                    && !(before.is_some() && *state == Some(MetadataChangeState::Set))
                    && !(after.is_some() && *state == Some(MetadataChangeState::Cleared))
            }
            Self::Review { reason, .. } => {
                trigger == DocumentUnreviewed || (trigger == DocumentReviewed && reason.is_none())
            }
            Self::Revision { number, source } => {
                number.is_none_or(|number| number > 0)
                    && (trigger == DocumentRevisionCreated
                        || (trigger == DocumentRevisionRestored
                            && number.is_none_or(|number| number > 1)
                            && source.is_none_or(|source| {
                                source == WorkflowRevisionSource::Restoration
                            })))
            }
        };
        if !valid {
            return Err(Error::Validation(
                "These document conditions are unavailable for the selected trigger".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn matches(&self, event: &DomainEventEnvelope) -> bool {
        let Some(trigger) = WorkflowTrigger::parse(event.event_type()) else {
            return false;
        };
        if self.validate_trigger(trigger).is_err() {
            return false;
        }
        let data = event.data();
        match self {
            Self::Date {
                before,
                after,
                state,
            } => {
                let Some(previous) = field::<Option<DocumentDate>>(data, "previous_document_date")
                else {
                    return false;
                };
                let Some(current) = field::<Option<DocumentDate>>(data, "document_date") else {
                    return false;
                };
                let change = match (previous, current) {
                    (None, Some(_)) => MetadataChangeState::Set,
                    (Some(before), Some(after)) if before != after => MetadataChangeState::Replaced,
                    (Some(_), None) => MetadataChangeState::Cleared,
                    _ => return false,
                };
                before.is_none_or(|date| previous == Some(date))
                    && after.is_none_or(|date| current == Some(date))
                    && state.is_none_or(|state| state == change)
            }
            Self::Review { origin, reason } => {
                let origin_field = if trigger == WorkflowTrigger::DocumentReviewed {
                    "review_origin"
                } else {
                    "previous_review_origin"
                };
                origin.is_none_or(|origin| field(data, origin_field) == Some(origin))
                    && reason.is_none_or(|reason| field(data, "reason") == Some(reason))
            }
            Self::Revision { number, source } => {
                number.is_none_or(|number| field(data, "revision_number") == Some(number))
                    && source.is_none_or(|source| revision_source(data) == Some(source))
            }
        }
    }
}

fn field<'a, T: Deserialize<'a>>(data: &'a Value, key: &str) -> Option<T> {
    T::deserialize(data.get(key)?).ok()
}

fn revision_source(data: &Value) -> Option<WorkflowRevisionSource> {
    let previous = field::<Option<Id>>(data, "previous_revision_id")?;
    let restored = field::<Option<Id>>(data, "restored_from_revision_id")?;
    match (previous, restored) {
        (None, None) => Some(WorkflowRevisionSource::Upload),
        (Some(_), None) => Some(WorkflowRevisionSource::Replacement),
        (Some(_), Some(_)) => Some(WorkflowRevisionSource::Restoration),
        (None, Some(_)) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{VaultId, WorkflowEventConditions, WorkflowTriggerRegistry};
    use crate::{DomainEventSubject, EventAudience};
    use serde_json::json;

    fn occurrence(trigger: WorkflowTrigger, data: Value) -> DomainEventEnvelope {
        let document = Id::new_v4();
        DomainEventEnvelope::new(
            trigger.as_str(),
            VaultId::new(),
            DomainEventSubject {
                subject_type: "document".into(),
                id: document,
            },
            Some(document),
            EventAudience::Vault,
            data,
        )
        .unwrap()
    }

    #[test]
    fn date_predicate_compares_the_captured_before_and_after_values() {
        let trigger = WorkflowTrigger::DocumentDateChanged;
        let conditions: WorkflowEventConditions = serde_json::from_value(json!({
            "document_change": { "type": "date", "before": "2024-02-29", "state": "cleared" }
        }))
        .unwrap();
        let condition = conditions.document_change.as_ref().unwrap();
        assert!(condition.validate_trigger(trigger).is_ok());
        assert!(condition
            .validate_trigger(WorkflowTrigger::DocumentUpdated)
            .is_err());
        assert!(conditions.matches(&occurrence(
            trigger,
            json!({
                "previous_document_date": "2024-02-29", "document_date": null
            })
        )));
        assert!(!conditions.matches(&occurrence(
            trigger,
            json!({
                "previous_document_date": "2024-02-28", "document_date": null
            })
        )));
        assert!(!conditions.matches(&occurrence(
            trigger,
            json!({
                "previous_document_date": "2024-02-29"
            })
        )));
    }

    #[test]
    fn review_predicate_distinguishes_unreview_reason_and_previous_origin() {
        let trigger = WorkflowTrigger::DocumentUnreviewed;
        let condition: WorkflowDocumentConditions = serde_json::from_value(json!({
            "type": "review", "origin": "human", "reason": "file_replaced"
        }))
        .unwrap();
        assert!(condition
            .validate_trigger(WorkflowTrigger::DocumentReviewed)
            .is_err());
        assert!(condition.matches(&occurrence(
            trigger,
            json!({
                "previous_review_origin": "human", "reason": "file_replaced"
            })
        )));
        assert!(!condition.matches(&occurrence(
            trigger,
            json!({
                "previous_review_origin": "human", "reason": "file_restored"
            })
        )));
    }

    #[test]
    fn revision_predicate_distinguishes_restored_revisions_from_replacements() {
        let trigger = WorkflowTrigger::DocumentRevisionCreated;
        let replaced = Id::new_v4();
        let history = Id::new_v4();
        let condition = WorkflowDocumentConditions::Revision {
            number: Some(3),
            source: Some(WorkflowRevisionSource::Restoration),
        };
        assert!(condition.matches(&occurrence(trigger, json!({
            "revision_number": 3, "previous_revision_id": replaced, "restored_from_revision_id": history
        }))));
        assert!(!condition.matches(&occurrence(trigger, json!({
            "revision_number": 3, "previous_revision_id": replaced, "restored_from_revision_id": null
        }))));
        let fields = &WorkflowTriggerRegistry::built_in()
            .get(trigger.as_str())
            .unwrap()
            .condition_fields;
        assert!(fields
            .iter()
            .any(|field| field.key == "event.document_change.source"));
    }
}
