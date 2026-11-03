use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::models::{UserId, VaultId};

pub const DOCUMENT_METADATA_CHANGED: &str = "document.metadata.changed";
const MAX_EVENT_DATA_BYTES: usize = 65_536;
const MAX_METADATA_SNAPSHOT_BYTES: usize = 1_048_576;

#[derive(Default)]
struct ByteCount(usize);

impl std::io::Write for ByteCount {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainEventSubject {
    #[serde(rename = "type")]
    pub subject_type: String,
    pub id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventAudience {
    Vault,
    User { user_id: UserId },
    Owners,
}

/// Immutable facts captured by trusted producers for the durable outbox.
/// Deserialization restores persisted attribution, so request handlers must use
/// [`Self::new`] rather than accepting envelopes from mutation request bodies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "EnvelopeFields")]
pub struct DomainEventEnvelope {
    schema_version: u32,
    event_id: Uuid,
    mutation_id: Uuid,
    event_type: String,
    occurred_at: DateTime<Utc>,
    vault_id: VaultId,
    subject: DomainEventSubject,
    actor_id: Option<UserId>,
    correlation_id: Uuid,
    causation_id: Option<Uuid>,
    document_id: Option<Uuid>,
    audience: EventAudience,
    data: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeFields {
    schema_version: u32,
    event_id: Uuid,
    mutation_id: Uuid,
    event_type: String,
    occurred_at: DateTime<Utc>,
    vault_id: VaultId,
    subject: DomainEventSubject,
    actor_id: Option<UserId>,
    correlation_id: Uuid,
    causation_id: Option<Uuid>,
    document_id: Option<Uuid>,
    audience: EventAudience,
    data: Value,
}

impl DomainEventEnvelope {
    /// Capture a committed transition from server context and an allowlisted producer payload.
    ///
    /// Producers must select the data fields permitted for the audience. Invalid
    /// identifiers and mismatched document targets fail validation. Data is capped at 64 KiB,
    /// or 1 MiB for vault-visible document metadata transitions with complete value snapshots.
    pub fn new(
        event_type: impl Into<String>,
        vault_id: VaultId,
        subject: DomainEventSubject,
        document_id: Option<Uuid>,
        audience: EventAudience,
        data: Value,
    ) -> Result<Self> {
        let event_id = Uuid::new_v4();
        let lineage = crate::mutation::current_lineage();
        let envelope = Self {
            schema_version: 1,
            event_id,
            mutation_id: Uuid::new_v4(),
            event_type: event_type.into(),
            occurred_at: Utc::now(),
            vault_id,
            subject,
            actor_id: crate::mutation::current_actor(),
            correlation_id: lineage.map_or(event_id, |lineage| lineage.correlation_id),
            causation_id: lineage.and_then(|lineage| lineage.causation_id),
            document_id,
            audience,
            data,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    /// Group events emitted by one committed mutation.
    pub fn with_mutation(mut self, mutation_id: Uuid) -> Result<Self> {
        self.mutation_id = mutation_id;
        self.validate()?;
        Ok(self)
    }

    /// Carry a server-established causal chain into this transition.
    pub fn with_correlation(
        mut self,
        correlation_id: Uuid,
        causation_id: Option<Uuid>,
    ) -> Result<Self> {
        self.correlation_id = correlation_id;
        self.causation_id = causation_id;
        self.validate()?;
        Ok(self)
    }

    /// Reject unsupported versions, inconsistent targets, or malformed producer data.
    pub fn validate(&self) -> Result<()> {
        let invalid = |message: &str| Error::Validation(message.into());
        if self.schema_version != 1 {
            return Err(invalid("Unsupported domain event schema version"));
        }
        if self.event_id.is_nil()
            || self.mutation_id.is_nil()
            || self.subject.id.is_nil()
            || self.correlation_id.is_nil()
            || self.causation_id.is_some_and(|id| id.is_nil())
        {
            return Err(invalid("Domain event identifiers must not be nil"));
        }
        let valid_name = |name: &str| {
            !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
                && name.as_bytes()[0].is_ascii_lowercase()
        };
        let mut parts = self.event_type.split('.');
        if !valid_name(&self.subject.subject_type)
            || parts.next() != Some(self.subject.subject_type.as_str())
            || !parts.next().is_some_and(valid_name)
            || !parts.all(valid_name)
        {
            return Err(invalid(
                "Domain event type must match its subject and use subject.action syntax",
            ));
        }
        if self.subject.subject_type == "document" && self.document_id != Some(self.subject.id) {
            return Err(invalid(
                "Domain event document target does not match its subject",
            ));
        }
        if matches!(
            self.subject.subject_type.as_str(),
            "vault" | "membership" | "vault_member" | "vault_membership"
        ) && self.audience != EventAudience::Owners
        {
            return Err(invalid(
                "Vault and membership events require an owners audience",
            ));
        }
        if let EventAudience::User { user_id } = &self.audience {
            if user_id.as_str().trim().is_empty() {
                return Err(invalid("Domain event audience requires a user"));
            }
        }
        let max_data_bytes = if self.event_type == DOCUMENT_METADATA_CHANGED
            && self.audience == EventAudience::Vault
        {
            MAX_METADATA_SNAPSHOT_BYTES
        } else {
            MAX_EVENT_DATA_BYTES
        };
        let mut size = ByteCount::default();
        serde_json::to_writer(&mut size, &self.data).map_err(|e| invalid(&e.to_string()))?;
        if !self.data.is_object() || size.0 > max_data_bytes {
            return Err(invalid(&format!(
                "Domain event data must be an object of at most {max_data_bytes} bytes"
            )));
        }
        Ok(())
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }
    pub fn event_id(&self) -> Uuid {
        self.event_id
    }
    pub fn mutation_id(&self) -> Uuid {
        self.mutation_id
    }
    pub fn event_type(&self) -> &str {
        &self.event_type
    }
    pub fn occurred_at(&self) -> DateTime<Utc> {
        self.occurred_at
    }
    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }
    pub fn subject(&self) -> &DomainEventSubject {
        &self.subject
    }
    pub fn actor_id(&self) -> Option<&UserId> {
        self.actor_id.as_ref()
    }
    pub fn correlation_id(&self) -> Uuid {
        self.correlation_id
    }
    pub fn causation_id(&self) -> Option<Uuid> {
        self.causation_id
    }
    pub fn document_id(&self) -> Option<Uuid> {
        self.document_id
    }
    pub fn audience(&self) -> &EventAudience {
        &self.audience
    }
    pub fn data(&self) -> &Value {
        &self.data
    }
}

impl TryFrom<EnvelopeFields> for DomainEventEnvelope {
    type Error = Error;

    fn try_from(fields: EnvelopeFields) -> Result<Self> {
        let envelope = Self {
            schema_version: fields.schema_version,
            event_id: fields.event_id,
            mutation_id: fields.mutation_id,
            event_type: fields.event_type,
            occurred_at: fields.occurred_at,
            vault_id: fields.vault_id,
            subject: fields.subject,
            actor_id: fields.actor_id,
            correlation_id: fields.correlation_id,
            causation_id: fields.causation_id,
            document_id: fields.document_id,
            audience: fields.audience,
            data: fields.data,
        };
        envelope.validate()?;
        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mutation::{with_lineage, MutationLineage};
    use serde_json::json;

    fn envelope() -> DomainEventEnvelope {
        try_envelope().unwrap()
    }

    fn try_envelope() -> crate::Result<DomainEventEnvelope> {
        DomainEventEnvelope::new(
            "tag.created",
            VaultId::new(),
            DomainEventSubject {
                subject_type: "tag".into(),
                id: Uuid::new_v4(),
            },
            None,
            EventAudience::Vault,
            json!({"name": "receipts"}),
        )
    }

    #[test]
    fn constructor_defaults_to_a_new_causal_chain_without_lineage() {
        let event = envelope();
        assert_eq!(event.correlation_id(), event.event_id());
        assert_eq!(event.causation_id(), None);
    }

    #[tokio::test]
    async fn constructor_captures_trusted_server_lineage() {
        let lineage = MutationLineage {
            correlation_id: Uuid::from_u128(1),
            causation_id: Some(Uuid::from_u128(2)),
        };
        with_lineage(Some(lineage), async {
            let event = envelope();
            assert_eq!(event.correlation_id(), lineage.correlation_id);
            assert_eq!(event.causation_id(), lineage.causation_id);
        })
        .await;
    }

    #[tokio::test]
    async fn constructor_rejects_nil_correlation_from_scoped_lineage() {
        let lineage = MutationLineage {
            correlation_id: Uuid::nil(),
            causation_id: None,
        };
        with_lineage(Some(lineage), async {
            assert!(matches!(try_envelope(), Err(Error::Validation(_))));
        })
        .await;
    }

    #[tokio::test]
    async fn constructor_rejects_nil_causation_from_scoped_lineage() {
        let lineage = MutationLineage {
            correlation_id: Uuid::from_u128(1),
            causation_id: Some(Uuid::nil()),
        };
        with_lineage(Some(lineage), async {
            assert!(matches!(try_envelope(), Err(Error::Validation(_))));
        })
        .await;
    }

    #[test]
    fn persisted_envelope_retains_identity_and_occurrence() {
        let event = envelope();
        let decoded: DomainEventEnvelope =
            serde_json::from_value(serde_json::to_value(&event).unwrap()).unwrap();
        assert_eq!(decoded, event);
        assert_eq!(decoded.schema_version(), 1);
        assert_eq!(decoded.document_id(), None);
    }

    #[test]
    fn mutation_and_causation_survive_persistence_without_changing_event_identity() {
        let event = envelope();
        let event_id = event.event_id();
        let mutation_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4();
        let causation_id = Uuid::new_v4();
        let event = event
            .with_mutation(mutation_id)
            .unwrap()
            .with_correlation(correlation_id, Some(causation_id))
            .unwrap();
        let restored: DomainEventEnvelope =
            serde_json::from_value(serde_json::to_value(event).unwrap()).unwrap();
        assert_eq!(restored.event_id(), event_id);
        assert_eq!(restored.mutation_id(), mutation_id);
        assert_eq!(restored.correlation_id(), correlation_id);
        assert_eq!(restored.causation_id(), Some(causation_id));
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        let mut data = serde_json::to_value(envelope()).unwrap();
        data["schema_version"] = json!(2);
        assert!(serde_json::from_value::<DomainEventEnvelope>(data).is_err());
    }

    #[test]
    fn document_subject_must_match_its_document_target() {
        let mut data = serde_json::to_value(envelope()).unwrap();
        data["event_type"] = json!("document.updated");
        data["subject"]["type"] = json!("document");
        data["document_id"] = json!(Uuid::new_v4());
        assert!(serde_json::from_value::<DomainEventEnvelope>(data).is_err());
    }

    #[test]
    fn child_subject_can_retain_its_explicit_parent_document() {
        let document_id = Uuid::new_v4();
        let event = DomainEventEnvelope::new(
            "comment.created",
            VaultId::new(),
            DomainEventSubject {
                subject_type: "comment".into(),
                id: Uuid::new_v4(),
            },
            Some(document_id),
            EventAudience::Vault,
            json!({}),
        )
        .unwrap();
        assert_eq!(event.document_id(), Some(document_id));
    }

    #[test]
    fn vault_membership_events_cannot_use_a_vault_wide_audience() {
        let event = DomainEventEnvelope::new(
            "vault_membership.granted",
            VaultId::new(),
            DomainEventSubject {
                subject_type: "vault_membership".into(),
                id: Uuid::new_v4(),
            },
            None,
            EventAudience::Vault,
            json!({"role": "viewer"}),
        );
        assert!(event.is_err());
    }

    #[test]
    fn invalid_event_type_and_unbounded_data_are_rejected() {
        let mut data = serde_json::to_value(envelope()).unwrap();
        data["event_type"] = json!("document.created");
        assert!(serde_json::from_value::<DomainEventEnvelope>(data).is_err());
        let mut data = serde_json::to_value(envelope()).unwrap();
        data["data"] = json!({"name": "x".repeat(65537)});
        assert!(serde_json::from_value::<DomainEventEnvelope>(data).is_err());
    }

    #[test]
    fn metadata_changes_retain_maximum_long_text_before_and_after() {
        let document = Uuid::new_v4();
        let before = "\u{0001}".repeat(65_536);
        let after = "\u{0002}".repeat(65_536);
        let event = DomainEventEnvelope::new(
            DOCUMENT_METADATA_CHANGED,
            VaultId::new(),
            DomainEventSubject {
                subject_type: "document".into(),
                id: document,
            },
            Some(document),
            EventAudience::Vault,
            json!({"changes": [{"field_id": Uuid::new_v4(), "kind": "long_text",
                "before": {"kind": "long_text", "value": before},
                "after": {"kind": "long_text", "value": after}}]}),
        )
        .expect("valid long text must fit an immutable transition event");
        let bytes = serde_json::to_vec(&event).unwrap();
        let decoded: DomainEventEnvelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.data()["changes"][0]["before"]["value"], before);
        assert_eq!(decoded.data()["changes"][0]["after"]["value"], after);
    }

    #[test]
    fn metadata_snapshot_budget_is_bounded_on_construction_and_decode() {
        let document = Uuid::new_v4();
        let overhead = serde_json::to_vec(&json!({"text": ""})).unwrap().len();
        let payload = json!({"text": "x".repeat(1_048_576 - overhead)});
        assert_eq!(serde_json::to_vec(&payload).unwrap().len(), 1_048_576);
        let event = DomainEventEnvelope::new(
            DOCUMENT_METADATA_CHANGED,
            VaultId::new(),
            DomainEventSubject {
                subject_type: "document".into(),
                id: document,
            },
            Some(document),
            EventAudience::Vault,
            payload.clone(),
        )
        .unwrap();
        let encoded = serde_json::to_value(&event).unwrap();
        assert!(serde_json::from_value::<DomainEventEnvelope>(encoded.clone()).is_ok());
        let mut oversized = payload;
        oversized["text"] = json!("x".repeat(1_048_577 - overhead));
        assert!(DomainEventEnvelope::new(
            DOCUMENT_METADATA_CHANGED,
            event.vault_id(),
            event.subject().clone(),
            Some(document),
            EventAudience::Vault,
            oversized.clone(),
        )
        .is_err());
        let mut oversized_envelope = encoded;
        oversized_envelope["data"] = oversized;
        assert!(serde_json::from_value::<DomainEventEnvelope>(oversized_envelope).is_err());
    }

    #[test]
    fn extended_snapshot_budget_requires_exact_event_type_and_vault_audience() {
        let document = Uuid::new_v4();
        let event = DomainEventEnvelope::new(
            DOCUMENT_METADATA_CHANGED,
            VaultId::new(),
            DomainEventSubject {
                subject_type: "document".into(),
                id: document,
            },
            Some(document),
            EventAudience::Vault,
            json!({"text": "x".repeat(65_537)}),
        )
        .unwrap();
        let mut wrong_event = serde_json::to_value(&event).unwrap();
        wrong_event["event_type"] = json!("document.metadata.changed_extra");
        assert!(serde_json::from_value::<DomainEventEnvelope>(wrong_event).is_err());
        let mut wrong_audience = serde_json::to_value(&event).unwrap();
        wrong_audience["audience"] = serde_json::to_value(EventAudience::Owners).unwrap();
        assert!(serde_json::from_value::<DomainEventEnvelope>(wrong_audience).is_err());
    }

    #[tokio::test]
    async fn constructor_captures_server_actor() {
        let actor = UserId::from("alice");
        crate::mutation::with_actor(Some(actor.clone()), async {
            assert_eq!(envelope().actor_id(), Some(&actor));
        })
        .await;
        assert_eq!(envelope().actor_id(), None);
    }
}
