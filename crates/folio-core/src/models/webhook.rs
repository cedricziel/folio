//! The webhook payload: a public, versioned contract with 3rd parties
//!
//! Additive changes (new optional fields) keep `version` at 1. Removing or
//! re-typing a field requires a new version.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    Correspondent, Document, DocumentSource, DocumentType, Id, Tag, VaultId, WorkflowTrigger,
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
    pub source: DocumentSource,
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
    pub document: WebhookDocument,
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
            document,
            source_event: None,
        }
    }

    /// A payload with made-up content, for the "send test" action
    pub fn sample(event: WorkflowTrigger, base_url: &str) -> Self {
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
        Self::new(event, Utc::now(), document)
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
            source: document.source.clone(),
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
