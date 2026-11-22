use crate::models::{
    IngestResult, SourceHealthChange, WorkflowConditionField, WorkflowConditionType,
    WorkflowTarget, WorkflowTriggerDefinition,
};

/// Defines ingestion choices for the shared workflow registry.
pub fn workflow_trigger_definitions() -> Vec<WorkflowTriggerDefinition> {
    use WorkflowConditionType::{Boolean, Identifier, Text};

    let source_fields = || {
        vec![
            field("data.source_id", "Source ID", Identifier),
            field("data.source_kind", "Source kind", Text),
        ]
    };
    let items = [
        (IngestResult::Imported, "Item imported"),
        (IngestResult::Skipped, "Item skipped by a rule"),
        (IngestResult::Duplicate, "Duplicate item found"),
        (IngestResult::Failed, "Item import failed"),
    ];
    let health = [
        (SourceHealthChange::Failed, "Source became unavailable"),
        (SourceHealthChange::Recovered, "Source recovered"),
    ];
    items
        .into_iter()
        .map(|(outcome, label)| {
            let mut fields = source_fields();
            fields.extend([
                field("data.outcome", "Outcome", Text),
                field("data.reason_code", "Reason", Text),
                field("data.retryable", "Retryable", Boolean),
            ]);
            WorkflowTriggerDefinition {
                event_type: outcome.event_type().into(),
                label: label.into(),
                subject_type: "ingest".into(),
                target: WorkflowTarget::Event,
                condition_fields: fields,
            }
        })
        .chain(health.into_iter().map(|(change, label)| {
            let mut fields = source_fields();
            if change == SourceHealthChange::Failed {
                fields.push(field("data.reason_code", "Reason", Text));
            }
            WorkflowTriggerDefinition {
                event_type: change.event_type().into(),
                label: label.into(),
                subject_type: "source".into(),
                target: WorkflowTarget::Event,
                condition_fields: fields,
            }
        }))
        .collect()
}

fn field(key: &str, label: &str, value_type: WorkflowConditionType) -> WorkflowConditionField {
    WorkflowConditionField {
        key: key.into(),
        label: label.into(),
        value_type,
    }
}
