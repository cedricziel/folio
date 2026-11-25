//! The webhook payload: a public, versioned contract with 3rd parties
//!
//! Additive changes (new optional fields) keep `version` at 1. Removing or
//! re-typing a field requires a new version.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    Correspondent, Document, DocumentSource, DocumentType, Id, SourceKind, Tag, UserId, VaultId,
    VaultRole, ViewPresentation, ViewVisibility, WorkflowTrigger,
};

/// Current payload version
pub const PAYLOAD_VERSION: u32 = 1;

/// A related entity, by id and display name
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NamedRef {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,
    pub name: String,
}

/// Links to the document, absolute so receivers can follow them
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookUrls {
    /// JSON API resource
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "http://folio.local:3000/api/documents/0b9c0c3e-0000-0000-0000-000000000000"
        )
    )]
    pub api: String,
    /// Page in the web UI
    pub web: String,
    /// The original file
    pub download: String,
}

/// The document an event is about, as it was when the event happened
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookDocument {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,
    pub title: String,
    /// Original filename
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    /// File size in bytes; null when no file is stored
    pub size: Option<u64>,
    pub correspondent: Option<NamedRef>,
    pub document_type: Option<NamedRef>,
    pub tags: Vec<NamedRef>,
    /// Archive serial number; always null until Folio supports serial numbers
    pub asn: Option<i64>,
    /// How the document entered Folio, with folder or email details when known
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<DocumentSource>,
    pub created_at: DateTime<Utc>,
    pub urls: WebhookUrls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowEventIdentity {
    pub event_id: uuid::Uuid,
    pub mutation_id: uuid::Uuid,
    pub correlation_id: uuid::Uuid,
}

impl From<&crate::DomainEventEnvelope> for WorkflowEventIdentity {
    fn from(event: &crate::DomainEventEnvelope) -> Self {
        Self {
            event_id: event.event_id(),
            mutation_id: event.mutation_id(),
            correlation_id: event.correlation_id(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookEntry {
    pub entry_id: uuid::Uuid,
    pub document_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub entry_kind: super::EntryKind,
    pub transition: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub actor_id: Option<super::UserId>,
    pub causation_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookIngest {
    pub payload_version: u32,
    pub source_id: uuid::Uuid,
    pub source_kind: super::SourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<super::IngestResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookMetadata {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub subject_id: uuid::Uuid,
    pub document_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub actor_id: Option<super::UserId>,
    pub causation_id: Option<uuid::Uuid>,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookProcessing {
    pub payload_version: u32,
    pub stage: crate::processing_events::ProcessingEventStage,
    pub outcome: String,
    pub run_id: uuid::Uuid,
    pub revision_id: Option<uuid::Uuid>,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = crate::processing_events::ProcessingReason, required = false))]
    pub reason_code: Option<crate::processing_events::ProcessingReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_text: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookMembership {
    pub principal_type: String,
    pub principal_id: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub before_role: Option<VaultRole>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub role: Option<VaultRole>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookVaultActivity {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub subject_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub actor_id: Option<UserId>,
    pub causation_id: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WebhookMembership, required = false))]
    pub membership: Option<WebhookMembership>,
}

impl WebhookVaultActivity {
    pub(crate) fn from_occurrence(
        trigger: WorkflowTrigger,
        event: &crate::DomainEventEnvelope,
    ) -> Option<Self> {
        let (name, membership) = match event.subject().subject_type.as_str() {
            "vault" if event.data()["deleted_at"].is_null() => {
                (Some(event.data()["name"].as_str()?.to_owned()), None)
            }
            "vault_membership" => (
                None,
                Some(WebhookMembership::deserialize(event.data()).ok()?),
            ),
            _ => return None,
        };
        let activity = Self {
            vault_id: event.vault_id(),
            subject_id: event.subject().id,
            actor_id: event.actor_id().cloned(),
            causation_id: event.causation_id(),
            name,
            membership,
        };
        (event.audience() == &crate::EventAudience::Owners
            && event.event_type() == trigger.as_str()
            && event.document_id().is_none()
            && activity.matches_trigger(trigger))
        .then_some(activity)
    }

    pub fn matches_trigger(&self, trigger: WorkflowTrigger) -> bool {
        if matches!(
            trigger,
            WorkflowTrigger::VaultUpdated | WorkflowTrigger::VaultRestored
        ) {
            return self.subject_id == self.vault_id.0
                && self
                    .name
                    .as_ref()
                    .is_some_and(|name| !name.trim().is_empty())
                && self.membership.is_none();
        }
        let Some(member) = &self.membership else {
            return false;
        };
        let expected_subject = Id::new_v5(
            &self.vault_id.0,
            format!("{}:{}", member.principal_type, member.principal_id).as_bytes(),
        );
        self.name.is_none()
            && matches!(member.principal_type.as_str(), "user" | "group")
            && !member.principal_id.trim().is_empty()
            && self.subject_id == expected_subject
            && match (trigger, member.before_role, member.role) {
                (WorkflowTrigger::MembershipGranted, None, Some(_))
                | (WorkflowTrigger::MembershipRevoked, Some(_), None) => true,
                (WorkflowTrigger::MembershipRoleChanged, Some(before), Some(after)) => {
                    before != after
                }
                _ => false,
            }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookSavedView {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub saved_view_id: uuid::Uuid,
    pub name: String,
    pub visibility: ViewVisibility,
    pub presentation: ViewPresentation,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub actor_id: Option<UserId>,
    pub causation_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookClassification {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub subject_id: uuid::Uuid,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub actor_id: Option<UserId>,
    pub causation_id: Option<uuid::Uuid>,
}

impl WebhookClassification {
    pub(crate) fn from_occurrence(
        trigger: WorkflowTrigger,
        event: &crate::DomainEventEnvelope,
    ) -> Option<Self> {
        #[derive(Deserialize)]
        struct ClassificationData {
            id: Id,
            vault_id: VaultId,
            name: String,
            color: Option<String>,
            deleted_at: Option<DateTime<Utc>>,
        }

        let subject = trigger.classification_subject()?;
        if event.event_type() != trigger.as_str()
            || event.subject().subject_type != subject
            || event.audience() != &crate::EventAudience::Vault
            || event.document_id().is_some()
        {
            return None;
        }
        let data = ClassificationData::deserialize(event.data()).ok()?;
        if data.id != event.subject().id
            || data.vault_id != event.vault_id()
            || data.deleted_at.is_some() != trigger.as_str().ends_with(".deleted")
        {
            return None;
        }
        Some(Self {
            vault_id: data.vault_id,
            subject_id: data.id,
            name: data.name,
            color: data.color.filter(|_| subject == "tag"),
            actor_id: event.actor_id().cloned(),
            causation_id: event.causation_id(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookSource {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub source_id: uuid::Uuid,
    pub name: String,
    pub kind: SourceKind,
    pub enabled: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub actor_id: Option<UserId>,
    pub causation_id: Option<uuid::Uuid>,
}

impl WebhookSource {
    pub(crate) fn from_occurrence(
        trigger: WorkflowTrigger,
        event: &crate::DomainEventEnvelope,
    ) -> Option<Self> {
        #[derive(Deserialize)]
        struct SourceData {
            name: String,
            kind: SourceKind,
            enabled: bool,
        }

        if !trigger.is_source_lifecycle()
            || event.event_type() != trigger.as_str()
            || event.subject().subject_type != "source"
            || event.audience() != &crate::EventAudience::Owners
            || event.document_id().is_some()
        {
            return None;
        }
        let data = SourceData::deserialize(event.data()).ok()?;
        Some(Self {
            vault_id: event.vault_id(),
            source_id: event.subject().id,
            name: data.name,
            kind: data.kind,
            enabled: data.enabled,
            actor_id: event.actor_id().cloned(),
            causation_id: event.causation_id(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookWorkflow {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub name: String,
    pub enabled: bool,
    pub trigger: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub actor_id: Option<super::UserId>,
    pub causation_id: Option<uuid::Uuid>,
}

impl WebhookWorkflow {
    pub(crate) fn from_envelope(event: &crate::DomainEventEnvelope) -> Option<Self> {
        if event.subject().subject_type != "workflow" || event.document_id().is_some() {
            return None;
        }
        let data = event.data();
        Some(Self {
            id: event.subject().id,
            vault_id: event.vault_id(),
            name: data.get("name")?.as_str()?.to_owned(),
            enabled: data.get("enabled")?.as_bool()?,
            trigger: data.get("trigger")?.as_str()?.to_owned(),
            actor_id: event.actor_id().cloned(),
            causation_id: event.causation_id(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookWorkflowRun {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub workflow_id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub causation_id: Option<uuid::Uuid>,
}

impl WebhookWorkflowRun {
    pub(crate) fn from_envelope(event: &crate::DomainEventEnvelope) -> Option<Self> {
        let id = event.subject().id;
        if event.subject().subject_type != "workflow_run"
            || event.mutation_id() != id
            || event.document_id().is_some()
            || event.actor_id().is_some()
        {
            return None;
        }
        let workflow_id = Id::parse_str(event.data().get("workflow_id")?.as_str()?).ok()?;
        Some(Self {
            id,
            workflow_id,
            vault_id: event.vault_id(),
            causation_id: event.causation_id(),
        })
    }
}

/// Body of every webhook delivery
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookPayload {
    /// Event name, for example `document.uploaded`
    pub event: WorkflowTrigger,
    /// Payload schema version
    #[cfg_attr(feature = "openapi", schema(example = 1))]
    pub version: u32,
    pub occurred_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<WebhookDocument>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<WebhookEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest: Option<WebhookIngest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<WebhookMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WebhookProcessing, required = false))]
    pub processing: Option<WebhookProcessing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WebhookVaultActivity, required = false))]
    pub vault_activity: Option<WebhookVaultActivity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WebhookSource, required = false))]
    pub source: Option<WebhookSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WebhookClassification, required = false))]
    pub classification: Option<WebhookClassification>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WebhookSavedView, required = false))]
    pub saved_view: Option<WebhookSavedView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WebhookWorkflow, required = false))]
    pub workflow: Option<WebhookWorkflow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WebhookWorkflowRun, required = false))]
    pub workflow_run: Option<WebhookWorkflowRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event: Option<WorkflowEventIdentity>,
}

impl WebhookPayload {
    fn for_event(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            event,
            version: PAYLOAD_VERSION,
            occurred_at,
            document: None,
            entry: None,
            ingest: None,
            metadata: None,
            processing: None,
            vault_activity: None,
            source: None,
            classification: None,
            saved_view: None,
            workflow: None,
            workflow_run: None,
            source_event,
        }
    }

    pub fn new(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        document: WebhookDocument,
    ) -> Self {
        Self {
            document: Some(document),
            ..Self::for_event(event, occurred_at, None)
        }
    }

    pub fn for_entry(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        entry: WebhookEntry,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            entry: Some(entry),
            ..Self::for_event(event, occurred_at, source_event)
        }
    }

    pub fn for_ingest(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        ingest: WebhookIngest,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            ingest: Some(ingest),
            ..Self::for_event(event, occurred_at, source_event)
        }
    }

    pub fn for_metadata(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        metadata: WebhookMetadata,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            metadata: Some(metadata),
            ..Self::for_event(event, occurred_at, source_event)
        }
    }

    pub fn for_vault_activity(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        vault_activity: WebhookVaultActivity,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            vault_activity: Some(vault_activity),
            ..Self::for_event(event, occurred_at, source_event)
        }
    }

    pub fn for_saved_view(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        saved_view: WebhookSavedView,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            saved_view: Some(saved_view),
            ..Self::for_event(event, occurred_at, source_event)
        }
    }

    pub fn for_classification(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        classification: WebhookClassification,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            classification: Some(classification),
            ..Self::for_event(event, occurred_at, source_event)
        }
    }

    pub fn for_source(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        source: WebhookSource,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            source: Some(source),
            ..Self::for_event(event, occurred_at, source_event)
        }
    }

    pub(crate) fn for_workflow_run(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        outcome: WebhookWorkflowRun,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            workflow_run: Some(outcome),
            ..Self::for_event(event, occurred_at, source_event)
        }
    }

    pub(crate) fn for_workflow_definition(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        workflow: WebhookWorkflow,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            workflow: Some(workflow),
            ..Self::for_event(event, occurred_at, source_event)
        }
    }

    pub fn for_clock(occurred_at: DateTime<Utc>) -> Self {
        Self::for_event(WorkflowTrigger::Scheduled, occurred_at, None)
    }

    /// A payload with made-up content, for the "send test" action
    pub fn sample(event: WorkflowTrigger, base_url: &str) -> Self {
        if event == WorkflowTrigger::Scheduled {
            return Self::for_clock(Utc::now());
        }
        if event.is_vault_activity() {
            let vault_id = VaultId(Id::nil());
            let membership = match event {
                WorkflowTrigger::MembershipGranted => Some((None, Some(VaultRole::Viewer))),
                WorkflowTrigger::MembershipRoleChanged => {
                    Some((Some(VaultRole::Viewer), Some(VaultRole::Editor)))
                }
                WorkflowTrigger::MembershipRevoked => Some((Some(VaultRole::Editor), None)),
                _ => None,
            };
            return Self::for_vault_activity(
                event,
                Utc::now(),
                WebhookVaultActivity {
                    vault_id,
                    subject_id: membership
                        .map_or(vault_id.0, |_| Id::new_v5(&vault_id.0, b"user:sample-user")),
                    actor_id: None,
                    causation_id: None,
                    name: membership.is_none().then(|| "Sample vault".into()),
                    membership: membership.map(|(before_role, role)| WebhookMembership {
                        principal_type: "user".into(),
                        principal_id: "sample-user".into(),
                        before_role,
                        role,
                    }),
                },
                None,
            );
        }
        if event.is_saved_view() {
            return Self::for_saved_view(
                event,
                Utc::now(),
                WebhookSavedView {
                    vault_id: VaultId(Id::nil()),
                    saved_view_id: Id::nil(),
                    name: "Sample shared view".into(),
                    visibility: ViewVisibility::Shared,
                    presentation: ViewPresentation::List,
                    actor_id: None,
                    causation_id: None,
                },
                None,
            );
        }
        if let Some(subject) = event.classification_subject() {
            return Self::for_classification(
                event,
                Utc::now(),
                WebhookClassification {
                    vault_id: VaultId(Id::nil()),
                    subject_id: Id::nil(),
                    name: match subject {
                        "tag" => "Finance",
                        "correspondent" => "Stadtwerke Köln",
                        _ => "Invoice",
                    }
                    .into(),
                    color: (subject == "tag").then(|| "#435e69".into()),
                    actor_id: None,
                    causation_id: None,
                },
                None,
            );
        }
        if event.is_source_lifecycle() {
            return Self::for_source(
                event,
                Utc::now(),
                WebhookSource {
                    vault_id: VaultId(Id::nil()),
                    source_id: Id::nil(),
                    name: "Sample managed source".into(),
                    kind: SourceKind::Folder,
                    enabled: false,
                    actor_id: None,
                    causation_id: None,
                },
                None,
            );
        }
        if event.is_workflow_definition() {
            return Self::for_workflow_definition(
                event,
                Utc::now(),
                WebhookWorkflow {
                    id: Id::nil(),
                    vault_id: VaultId(Id::nil()),
                    name: "File invoices".into(),
                    enabled: event != WorkflowTrigger::WorkflowDisabled,
                    trigger: WorkflowTrigger::DocumentUploaded.as_str().into(),
                    actor_id: None,
                    causation_id: None,
                },
                None,
            );
        }
        if event.terminal_run_status().is_some() {
            return Self::for_workflow_run(
                event,
                Utc::now(),
                WebhookWorkflowRun {
                    id: Id::nil(),
                    workflow_id: Id::nil(),
                    vault_id: VaultId(Id::nil()),
                    causation_id: None,
                },
                None,
            );
        }
        if event.is_custom_metadata() {
            let id = Id::nil();
            let data = match event.as_str().split('.').next() {
                Some("metadata_field") => serde_json::json!({
                    "field_id": id, "name": "Approved", "kind": "boolean",
                    "required": false, "unique": false, "deleted_at": null
                }),
                Some("metadata_option") => serde_json::json!({
                    "option_id": id, "field_id": id, "label": "Pending",
                    "position": 0, "deleted_at": null
                }),
                _ => serde_json::json!({
                    "revision": id,
                    "changes": [{
                        "field_id": id, "kind": "boolean", "before": null,
                        "after": {"kind": "boolean", "value": false}
                    }]
                }),
            };
            return Self::for_metadata(
                event,
                Utc::now(),
                WebhookMetadata {
                    vault_id: VaultId(id),
                    subject_id: id,
                    document_id: (event.target() == super::WorkflowTarget::Document).then_some(id),
                    data,
                    actor_id: None,
                    causation_id: None,
                },
                None,
            );
        }
        if let Some((entry_kind, transition)) = event.entry_transition() {
            return Self::for_entry(
                event,
                Utc::now(),
                WebhookEntry {
                    entry_id: Id::nil(),
                    document_id: Id::nil(),
                    vault_id: VaultId(Id::nil()),
                    entry_kind,
                    transition: transition.into(),
                    actor_id: None,
                    causation_id: None,
                },
                None,
            );
        }
        if event.is_ingest_outcome() {
            use super::IngestResult;
            let outcome = match event {
                WorkflowTrigger::IngestImported => Some(IngestResult::Imported),
                WorkflowTrigger::IngestSkipped => Some(IngestResult::Skipped),
                WorkflowTrigger::IngestDuplicate => Some(IngestResult::Duplicate),
                WorkflowTrigger::IngestFailed => Some(IngestResult::Failed),
                _ => None,
            };
            let reason_code = outcome.map(IngestResult::reason_code).or_else(|| {
                (event == WorkflowTrigger::SourceFailed).then_some("source_unavailable")
            });
            return Self::for_ingest(
                event,
                Utc::now(),
                WebhookIngest {
                    payload_version: 1,
                    source_id: Id::nil(),
                    source_kind: super::SourceKind::Folder,
                    item_id: outcome.map(|_| Id::nil()),
                    attempt_id: outcome.map(|_| Id::nil()),
                    outcome,
                    reason_code: reason_code.map(str::to_owned),
                    retryable: outcome.map(IngestResult::retryable),
                },
                None,
            );
        }
        let mut doc = Document::new(VaultId::new(), "Sample invoice")
            .with_filename("invoice-2025-03.pdf")
            .with_mime_type("application/pdf");
        doc.id = Id::nil();
        let sender = Correspondent::new(doc.vault_id, "Stadtwerke Köln");
        let kind = DocumentType::new(doc.vault_id, "Invoice");
        let tag = Tag::new(doc.vault_id, "finance");
        let document = WebhookDocument::from_parts(
            &doc,
            &[tag],
            Some(&sender),
            Some(&kind),
            Some(48_213),
            base_url,
        );
        let now = Utc::now();
        let mut payload = Self::new(event, now, document);
        if let Some((stage, outcome)) = event
            .as_str()
            .strip_prefix("document.processing.")
            .and_then(|name| name.split_once('.'))
        {
            use crate::processing_events::{ProcessingEventStage, ProcessingReason};
            let stage = match stage {
                "ocr" => ProcessingEventStage::Ocr,
                "embedding" => ProcessingEventStage::Embedding,
                "thumbnail" => ProcessingEventStage::Thumbnail,
                _ => return payload,
            };
            let reason_code = match outcome {
                "failed" | "retry_scheduled" => Some(ProcessingReason::ProviderFailed),
                "skipped" => Some(ProcessingReason::ProviderDisabled),
                _ => None,
            };
            if let Some(document) = &mut payload.document {
                document.source = None;
            }
            payload.processing = Some(WebhookProcessing {
                payload_version: 1,
                stage,
                outcome: outcome.into(),
                run_id: Id::nil(),
                revision_id: None,
                attempt: 1,
                reason_code,
                retry_at: (outcome == "retry_scheduled").then_some(now),
                has_text: (stage == ProcessingEventStage::Ocr && outcome == "completed")
                    .then_some(true),
            });
        }
        payload
    }
}

impl WebhookUrls {
    fn for_document(base_url: &str, id: Id) -> Self {
        let base = base_url.trim_end_matches('/');
        Self {
            api: format!("{base}/api/documents/{id}"),
            web: format!("{base}/documents/{id}"),
            download: format!("{base}/api/documents/{id}/download"),
        }
    }
}

impl WebhookDocument {
    /// Build a delivery from captured metadata without consulting mutable document rows.
    pub fn from_snapshot(
        snapshot: &crate::EventSnapshot,
        size: Option<u64>,
        base_url: &str,
    ) -> Self {
        Self {
            id: snapshot.document_id,
            title: snapshot.title.clone(),
            filename: snapshot.original_filename.clone(),
            mime_type: snapshot.mime_type.clone(),
            size,
            correspondent: snapshot
                .correspondent_id
                .zip(snapshot.correspondent_name.as_ref())
                .map(|(id, name)| NamedRef {
                    id,
                    name: name.clone(),
                }),
            document_type: snapshot
                .document_type_id
                .zip(snapshot.document_type_name.as_ref())
                .map(|(id, name)| NamedRef {
                    id,
                    name: name.clone(),
                }),
            tags: snapshot.tags.clone(),
            asn: None,
            source: snapshot.source.clone(),
            created_at: snapshot.created_at,
            urls: WebhookUrls::for_document(base_url, snapshot.document_id),
        }
    }

    pub fn from_parts(
        document: &Document,
        tags: &[Tag],
        correspondent: Option<&Correspondent>,
        document_type: Option<&DocumentType>,
        size: Option<u64>,
        base_url: &str,
    ) -> Self {
        let id = document.id;
        Self {
            id,
            title: document.title.clone(),
            filename: document.original_filename.clone(),
            mime_type: document.mime_type.clone(),
            size,
            correspondent: correspondent.map(|c| NamedRef {
                id: c.id,
                name: c.name.clone(),
            }),
            document_type: document_type.map(|t| NamedRef {
                id: t.id,
                name: t.name.clone(),
            }),
            tags: tags
                .iter()
                .map(|t| NamedRef {
                    id: t.id,
                    name: t.name.clone(),
                })
                .collect(),
            asn: None,
            source: Some(document.source.clone()),
            created_at: document.created_at,
            urls: WebhookUrls::for_document(base_url, id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_matches_the_documented_shape() {
        let payload = WebhookPayload::sample(
            WorkflowTrigger::DocumentUploaded,
            "http://folio.local:3000/",
        );
        let json = serde_json::to_value(&payload).unwrap();

        assert_eq!(json["event"], "document.uploaded");
        assert_eq!(json["version"], 1);
        assert!(json["occurred_at"].is_string());
        let doc = &json["document"];
        for key in [
            "id",
            "title",
            "filename",
            "mime_type",
            "size",
            "correspondent",
            "document_type",
            "tags",
            "asn",
            "source",
            "created_at",
            "urls",
        ] {
            assert!(doc.get(key).is_some(), "missing {key}");
        }
        assert_eq!(doc["tags"][0]["name"], "finance");
        assert_eq!(
            doc["urls"]["download"],
            "http://folio.local:3000/api/documents/00000000-0000-0000-0000-000000000000/download"
        );
        assert!(doc["asn"].is_null());
    }
}
