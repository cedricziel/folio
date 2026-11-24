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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkflowTagChangeState {
    Added,
    Removed,
    Replaced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkflowDocumentConditions {
    Title {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = String, required = false))]
        before: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = String, required = false))]
        after: Option<String>,
    },
    Tags {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, required = false))]
        added_id: Option<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, required = false))]
        removed_id: Option<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = WorkflowTagChangeState, required = false))]
        state: Option<WorkflowTagChangeState>,
    },
    Correspondent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, required = false))]
        before_id: Option<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, required = false))]
        after_id: Option<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = MetadataChangeState, required = false))]
        state: Option<MetadataChangeState>,
    },
    DocumentType {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, required = false))]
        before_id: Option<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid, required = false))]
        after_id: Option<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "openapi", schema(value_type = MetadataChangeState, required = false))]
        state: Option<MetadataChangeState>,
    },
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
            Self::Title { .. } => trigger == DocumentTitleChanged,
            Self::Tags {
                added_id,
                removed_id,
                state,
            } => {
                trigger == DocumentTagsChanged
                    && !(added_id.is_some() && *state == Some(WorkflowTagChangeState::Removed))
                    && !(removed_id.is_some() && *state == Some(WorkflowTagChangeState::Added))
                    && (added_id.is_none() || added_id != removed_id)
            }
            Self::Correspondent {
                before_id,
                after_id,
                state,
            } => {
                trigger == DocumentCorrespondentChanged
                    && valid_assignment(before_id, after_id, *state)
            }
            Self::DocumentType {
                before_id,
                after_id,
                state,
            } => trigger == DocumentTypeChanged && valid_assignment(before_id, after_id, *state),
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
            Self::Title { before, after } => {
                let (Some(previous), Some(current)) =
                    (field::<&str>(data, "before"), field::<&str>(data, "after"))
                else {
                    return false;
                };
                previous != current
                    && before.as_deref().is_none_or(|value| previous == value)
                    && after.as_deref().is_none_or(|value| current == value)
            }
            Self::Tags {
                added_id,
                removed_id,
                state,
            } => {
                let (Some(added), Some(removed)) = (
                    field::<Vec<Id>>(data, "added_ids"),
                    field::<Vec<Id>>(data, "removed_ids"),
                ) else {
                    return false;
                };
                let change = match (added.is_empty(), removed.is_empty()) {
                    (false, true) => WorkflowTagChangeState::Added,
                    (true, false) => WorkflowTagChangeState::Removed,
                    (false, false) => WorkflowTagChangeState::Replaced,
                    (true, true) => return false,
                };
                added_id.is_none_or(|id| added.contains(&id))
                    && removed_id.is_none_or(|id| removed.contains(&id))
                    && state.is_none_or(|state| state == change)
            }
            Self::Correspondent {
                before_id,
                after_id,
                state,
            }
            | Self::DocumentType {
                before_id,
                after_id,
                state,
            } => {
                let (Some(previous), Some(current)) = (
                    field::<Option<Id>>(data, "before_id"),
                    field::<Option<Id>>(data, "after_id"),
                ) else {
                    return false;
                };
                matches_assignment(previous, current, *before_id, *after_id, *state)
            }
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
                matches_assignment(previous, current, *before, *after, *state)
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

fn valid_assignment<T: PartialEq>(
    before: &Option<T>,
    after: &Option<T>,
    state: Option<MetadataChangeState>,
) -> bool {
    !(before.is_some() && state == Some(MetadataChangeState::Set)
        || after.is_some() && state == Some(MetadataChangeState::Cleared)
        || before.is_some() && before == after)
}

fn matches_assignment<T: Copy + PartialEq>(
    previous: Option<T>,
    current: Option<T>,
    before: Option<T>,
    after: Option<T>,
    state: Option<MetadataChangeState>,
) -> bool {
    let change = match (previous, current) {
        (None, Some(_)) => MetadataChangeState::Set,
        (Some(before), Some(after)) if before != after => MetadataChangeState::Replaced,
        (Some(_), None) => MetadataChangeState::Cleared,
        _ => return false,
    };
    before.is_none_or(|value| previous == Some(value))
        && after.is_none_or(|value| current == Some(value))
        && state.is_none_or(|state| state == change)
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
    fn title_predicate_compares_exact_captured_text() {
        let trigger = WorkflowTrigger::DocumentTitleChanged;
        let condition = WorkflowDocumentConditions::Title {
            before: Some("Invoice A".into()),
            after: Some("Invoice B".into()),
        };
        assert!(condition.matches(&occurrence(
            trigger,
            json!({"before": "Invoice A", "after": "Invoice B"})
        )));
        assert!(!condition.matches(&occurrence(
            trigger,
            json!({"before": "invoice A", "after": "Invoice B"})
        )));
        assert!(!condition.matches(&occurrence(
            WorkflowTrigger::DocumentUpdated,
            json!({"before": "Invoice A", "after": "Invoice B"})
        )));
    }

    #[test]
    fn assignment_predicates_distinguish_setting_replacing_and_clearing() {
        let old_id = Id::new_v4();
        let new_id = Id::new_v4();
        let correspondent = WorkflowTrigger::DocumentCorrespondentChanged;
        let document_type = WorkflowTrigger::DocumentTypeChanged;
        let assign = WorkflowDocumentConditions::Correspondent {
            before_id: None,
            after_id: Some(new_id),
            state: Some(MetadataChangeState::Set),
        };
        assert!(assign.matches(&occurrence(
            correspondent,
            json!({"before_id": null, "after_id": new_id})
        )));
        assert!(!assign.matches(&occurrence(correspondent, json!({"after_id": new_id}))));
        assert!(!assign.matches(&occurrence(
            correspondent,
            json!({"before_id": old_id, "after_id": new_id})
        )));
        let replace = WorkflowDocumentConditions::DocumentType {
            before_id: Some(old_id),
            after_id: Some(new_id),
            state: Some(MetadataChangeState::Replaced),
        };
        assert!(replace.matches(&occurrence(
            document_type,
            json!({"before_id": old_id, "after_id": new_id})
        )));
        assert!(!replace.matches(&occurrence(
            correspondent,
            json!({"before_id": old_id, "after_id": new_id})
        )));
        let clear = WorkflowDocumentConditions::DocumentType {
            before_id: Some(old_id),
            after_id: None,
            state: Some(MetadataChangeState::Cleared),
        };
        assert!(clear.matches(&occurrence(
            document_type,
            json!({"before_id": old_id, "after_id": null})
        )));
        assert!(!clear.matches(&occurrence(
            document_type,
            json!({"before_id": old_id, "after_id": old_id})
        )));
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
