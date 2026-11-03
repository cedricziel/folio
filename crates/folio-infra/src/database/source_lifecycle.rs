use folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience};
use folio_core::models::IngestionSourceDef;
use folio_core::{Error, Result};

pub(super) fn source_event(
    source: &IngestionSourceDef,
    action: &str,
) -> Result<DomainEventEnvelope> {
    let vault = source
        .vault_id
        .ok_or_else(|| Error::Validation("Managed source needs a vault".into()))?;
    DomainEventEnvelope::new(
        format!("source.{action}"),
        vault,
        DomainEventSubject {
            subject_type: "source".into(),
            id: source.id,
        },
        None,
        EventAudience::Owners,
        serde_json::json!({"name": source.name, "kind": source.config.kind(), "enabled": source.enabled}),
    )
}
