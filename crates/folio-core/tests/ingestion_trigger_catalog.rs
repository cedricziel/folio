use folio_core::ingestion_triggers::workflow_trigger_definitions;
use folio_core::models::{
    Id, WorkflowAction, WorkflowConditionType, WorkflowTarget, WorkflowTriggerRegistry,
};

#[test]
fn outcome_triggers_accept_webhooks_without_document_actions_or_skip_vetoes() {
    let registry = WorkflowTriggerRegistry::new(workflow_trigger_definitions()).unwrap();
    let events = [
        ("ingest.imported", "Item imported"),
        ("ingest.skipped", "Item skipped by a rule"),
        ("ingest.duplicate", "Duplicate item found"),
        ("ingest.failed", "Item import failed"),
        ("source.failed", "Source became unavailable"),
        ("source.recovered", "Source recovered"),
    ];
    assert_eq!(registry.definitions().len(), events.len());
    for (name, label) in events {
        let trigger = registry.get(name).unwrap();
        assert_eq!(trigger.label, label);
        assert_eq!(trigger.target, WorkflowTarget::Event);
        assert!(registry
            .validate_actions(
                &[name],
                &[WorkflowAction::Webhook {
                    url: "https://example.test/hook".into(),
                    method: Default::default(),
                    headers: Default::default(),
                    secret: None,
                }],
            )
            .is_ok());
        assert!(registry
            .validate_actions(&[name], &[WorkflowAction::SkipImport])
            .is_err());
        assert!(registry
            .validate_actions(
                &[name],
                &[WorkflowAction::AddTags {
                    tag_ids: vec![Id::new_v4()],
                }],
            )
            .is_err());
    }
    assert!(registry.get("ingest.received").is_none());
}

#[test]
fn condition_choices_match_the_fields_available_for_each_event() {
    use WorkflowConditionType::{Boolean, Identifier, Text};

    let registry = WorkflowTriggerRegistry::new(workflow_trigger_definitions()).unwrap();
    let fields = |event| {
        registry
            .get(event)
            .unwrap()
            .condition_fields
            .iter()
            .map(|field| (field.key.as_str(), field.value_type))
            .collect::<Vec<_>>()
    };
    let source_fields = [("data.source_id", Identifier), ("data.source_kind", Text)];
    let item_fields = [
        ("data.outcome", Text),
        ("data.reason_code", Text),
        ("data.retryable", Boolean),
    ];
    for event in [
        "ingest.imported",
        "ingest.skipped",
        "ingest.duplicate",
        "ingest.failed",
    ] {
        assert_eq!(
            fields(event),
            [source_fields.as_slice(), item_fields.as_slice()].concat()
        );
    }
    assert_eq!(fields("source.recovered"), source_fields);
    assert_eq!(
        fields("source.failed"),
        [source_fields.as_slice(), &[("data.reason_code", Text)]].concat()
    );
}
