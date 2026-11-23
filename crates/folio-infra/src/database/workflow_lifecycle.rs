use folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
use folio_core::models::Workflow;
use folio_core::Result;

pub(super) fn workflow_event(workflow: &Workflow, action: &str) -> Result<DomainEventEnvelope> {
    DomainEventEnvelope::new(
        format!("workflow.{action}"),
        workflow.vault_id,
        DomainEventSubject {
            subject_type: "workflow".into(),
            id: workflow.id,
        },
        None,
        EventAudience::Owners,
        serde_json::json!({"name": workflow.name, "enabled": workflow.enabled, "trigger": workflow.trigger.as_str()}),
    )
}

pub(super) fn workflow_update_events(
    workflow: &Workflow,
    was_enabled: bool,
) -> Result<Vec<DomainEventEnvelope>> {
    let updated = workflow_event(workflow, "updated")?;
    if workflow.enabled == was_enabled {
        return Ok(vec![updated]);
    }

    let action = if workflow.enabled {
        "enabled"
    } else {
        "disabled"
    };
    let transition = workflow_event(workflow, action)?
        .with_mutation(updated.mutation_id())?
        .with_correlation(updated.correlation_id(), updated.causation_id())?;
    Ok(vec![updated, transition])
}
