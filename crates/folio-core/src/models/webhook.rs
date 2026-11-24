//! The webhook payload: a public, versioned contract with 3rd parties
//!
//! Additive changes (new optional fields) keep `version` at 1. Removing or
//! re-typing a field requires a new version.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    Correspondent, Document, DocumentSource, DocumentType, Id, Tag, UserId, VaultId,
    ViewPresentation, ViewVisibility, WorkflowTrigger,
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
    #[cfg_attr(feature = "openapi", schema(value_type = WebhookSavedView, required = false))]
    pub saved_view: Option<WebhookSavedView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event: Option<WorkflowEventIdentity>,
}

impl WebhookPayload {
    pub fn new(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        document: WebhookDocument,
    ) -> Self {
        Self {
            event,
            version: PAYLOAD_VERSION,
            occurred_at,
            document: Some(document),
            entry: None,
            ingest: None,
            metadata: None,
            processing: None,
            saved_view: None,
            source_event: None,
        }
    }

    pub fn for_entry(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        entry: WebhookEntry,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            event,
            version: PAYLOAD_VERSION,
            occurred_at,
            document: None,
            entry: Some(entry),
            ingest: None,
            metadata: None,
            processing: None,
            saved_view: None,
            source_event,
        }
    }

    pub fn for_ingest(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        ingest: WebhookIngest,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            event,
            version: PAYLOAD_VERSION,
            occurred_at,
            document: None,
            entry: None,
            ingest: Some(ingest),
            metadata: None,
            processing: None,
            saved_view: None,
            source_event,
        }
    }

    pub fn for_metadata(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        metadata: WebhookMetadata,
        source_event: Option<WorkflowEventIdentity>,
    ) -> Self {
        Self {
            event,
            version: PAYLOAD_VERSION,
            occurred_at,
            document: None,
            entry: None,
            ingest: None,
            metadata: Some(metadata),
            processing: None,
            saved_view: None,
            source_event,
        }
    }

    pub fn for_saved_view(
        event: WorkflowTrigger,
        occurred_at: DateTime<Utc>,
        saved_view: WebhookSavedView,
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
            saved_view: Some(saved_view),
            source_event,
        }
    }

    pub fn for_clock(occurred_at: DateTime<Utc>) -> Self {
        Self {
            event: WorkflowTrigger::Scheduled,
            version: PAYLOAD_VERSION,
            occurred_at,
            document: None,
            entry: None,
            ingest: None,
            metadata: None,
            processing: None,
            saved_view: None,
            source_event: None,
        }
    }

    /// A payload with made-up content, for the "send test" action
    pub fn sample(event: WorkflowTrigger, base_url: &str) -> Self {
        if event == WorkflowTrigger::Scheduled {
            return Self::for_clock(Utc::now());
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
