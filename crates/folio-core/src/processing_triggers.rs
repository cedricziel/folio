use crate::models::{
    WorkflowConditionField, WorkflowConditionType, WorkflowTarget, WorkflowTriggerDefinition,
};

/// Producer-owned choices for the shared workflow registry; this does not enable dispatch.
pub fn workflow_trigger_definitions() -> Vec<WorkflowTriggerDefinition> {
    let mut definitions = Vec::with_capacity(15);
    for (stage, label) in [
        ("ocr", "Text extraction"),
        ("embedding", "Search indexing"),
        ("thumbnail", "Preview generation"),
    ] {
        for (outcome, outcome_label) in [
            ("started", "started"),
            ("completed", "completed"),
            ("failed", "failed"),
            ("skipped", "skipped"),
            ("retry_scheduled", "retry scheduled"),
        ] {
            let mut fields = vec![
                field("stage", "Stage", WorkflowConditionType::Text),
                field("outcome", "Outcome", WorkflowConditionType::Text),
                field(
                    "run_id",
                    "Processing run",
                    WorkflowConditionType::Identifier,
                ),
                field("revision_id", "Revision", WorkflowConditionType::Identifier),
                field("attempt", "Attempt", WorkflowConditionType::Integer),
            ];
            if matches!(outcome, "failed" | "skipped" | "retry_scheduled") {
                fields.push(field("reason_code", "Reason", WorkflowConditionType::Text));
            }
            if outcome == "retry_scheduled" {
                fields.push(field(
                    "retry_at",
                    "Retry time",
                    WorkflowConditionType::Timestamp,
                ));
            }
            if stage == "ocr" && outcome == "completed" {
                fields.push(field(
                    "has_text",
                    "Text found",
                    WorkflowConditionType::Boolean,
                ));
            }
            definitions.push(WorkflowTriggerDefinition {
                event_type: format!("document.processing.{stage}.{outcome}"),
                label: format!("{label} {outcome_label}"),
                subject_type: "document".into(),
                target: WorkflowTarget::Document,
                condition_fields: fields,
            });
        }
    }
    definitions
}

fn field(key: &str, label: &str, value_type: WorkflowConditionType) -> WorkflowConditionField {
    WorkflowConditionField {
        key: format!("data.{key}"),
        label: label.into(),
        value_type,
    }
}
