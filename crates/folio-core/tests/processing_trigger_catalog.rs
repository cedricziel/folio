use folio_core::models::{WorkflowConditionType, WorkflowTarget, WorkflowTriggerRegistry};
use folio_core::processing_triggers::workflow_trigger_definitions;

#[test]
fn processing_catalog_registers_distinct_retry_and_terminal_failure_choices() {
    let registry = WorkflowTriggerRegistry::new(workflow_trigger_definitions()).unwrap();
    assert_eq!(registry.definitions().len(), 15);
    let retry = registry
        .get("document.processing.embedding.retry_scheduled")
        .unwrap();
    let failure = registry
        .get("document.processing.embedding.failed")
        .unwrap();
    assert_eq!(retry.label, "Search indexing retry scheduled");
    assert_eq!(failure.label, "Search indexing failed");
    assert_eq!(failure.target, WorkflowTarget::Document);
    assert!(retry
        .condition_fields
        .iter()
        .any(|field| field.key == "data.retry_at"
            && field.value_type == WorkflowConditionType::Timestamp));
    assert!(!failure
        .condition_fields
        .iter()
        .any(|field| field.key == "data.retry_at"));
}

#[test]
fn only_ocr_completion_offers_an_empty_text_condition() {
    let registry = WorkflowTriggerRegistry::new(workflow_trigger_definitions()).unwrap();
    let choices: Vec<_> = registry
        .definitions()
        .iter()
        .filter(|definition| {
            definition
                .condition_fields
                .iter()
                .any(|field| field.key == "data.has_text")
        })
        .map(|definition| definition.event_type.as_str())
        .collect();
    assert_eq!(choices, ["document.processing.ocr.completed"]);
}

#[test]
fn processing_choices_offer_bounded_identity_and_reason_fields() {
    let registry = WorkflowTriggerRegistry::new(workflow_trigger_definitions()).unwrap();
    let skipped = registry
        .get("document.processing.thumbnail.skipped")
        .unwrap();
    let fields: Vec<_> = skipped
        .condition_fields
        .iter()
        .map(|field| field.key.as_str())
        .collect();
    assert_eq!(
        fields,
        [
            "data.stage",
            "data.outcome",
            "data.run_id",
            "data.revision_id",
            "data.attempt",
            "data.reason_code"
        ]
    );
    let started = registry
        .get("document.processing.thumbnail.started")
        .unwrap();
    assert!(!started
        .condition_fields
        .iter()
        .any(|field| field.key == "data.reason_code"));
}
