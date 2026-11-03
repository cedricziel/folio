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
