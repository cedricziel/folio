//! Workflow triggers, conditions, and ordered actions

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{
    Document, DocumentDate, DocumentSource, Id, MetadataValue, SourceConditions, UserId, VaultId,
    WorkflowCadence, WorkflowEventConditions, WorkflowEventIdentity, WorkflowTarget,
    WorkflowTriggerRegistry,
};
use crate::error::{Error, Result};

/// Events a workflow can listen for
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkflowTrigger {
    /// A document was added
    #[serde(rename = "document.uploaded")]
    DocumentUploaded,
    /// Background processing (thumbnails, OCR, embedding) finished
    #[serde(rename = "document.processed")]
    DocumentProcessed,
    /// A document's metadata changed
    #[serde(rename = "document.updated")]
    DocumentUpdated,
    #[serde(rename = "document.title_changed")]
    DocumentTitleChanged,
    #[serde(rename = "document.tags_changed")]
    DocumentTagsChanged,
    #[serde(rename = "document.correspondent_changed")]
    DocumentCorrespondentChanged,
    #[serde(rename = "document.document_type_changed")]
    DocumentTypeChanged,
    #[serde(rename = "document_note.created")]
    DocumentNoteCreated,
    #[serde(rename = "document_note.updated")]
    DocumentNoteUpdated,
    #[serde(rename = "document_note.deleted")]
    DocumentNoteDeleted,
    #[serde(rename = "document_comment.created")]
    DocumentCommentCreated,
    #[serde(rename = "document_comment.updated")]
    DocumentCommentUpdated,
    #[serde(rename = "document_comment.deleted")]
    DocumentCommentDeleted,
    #[serde(rename = "saved_view.created")]
    SavedViewCreated,
    #[serde(rename = "saved_view.updated")]
    SavedViewUpdated,
    #[serde(rename = "saved_view.deleted")]
    SavedViewDeleted,
    #[serde(rename = "metadata_field.created")]
    MetadataFieldCreated,
    #[serde(rename = "metadata_field.updated")]
    MetadataFieldUpdated,
    #[serde(rename = "metadata_field.deleted")]
    MetadataFieldDeleted,
    #[serde(rename = "metadata_field.restored")]
    MetadataFieldRestored,
    #[serde(rename = "metadata_option.created")]
    MetadataOptionCreated,
    #[serde(rename = "metadata_option.updated")]
    MetadataOptionUpdated,
    #[serde(rename = "metadata_option.deleted")]
    MetadataOptionDeleted,
    #[serde(rename = "metadata_option.restored")]
    MetadataOptionRestored,
    #[serde(rename = "document.metadata.changed")]
    DocumentMetadataChanged,
    #[serde(rename = "document.moved_out")]
    DocumentMovedOut,
    #[serde(rename = "document.moved_in")]
    DocumentMovedIn,
    #[serde(rename = "document.trashed")]
    DocumentTrashed,
    #[serde(rename = "document.restored")]
    DocumentRestored,
    #[serde(rename = "document.purge_requested")]
    DocumentPurgeRequested,
    #[serde(rename = "document.purged")]
    DocumentPurged,
    /// A document was deleted
    #[serde(rename = "document.deleted")]
    DocumentDeleted,
    /// A file or email attachment was found by an ingestion source and is about
    /// to be imported; rules here can discard it
    #[serde(rename = "ingest.received")]
    IngestReceived,
    #[serde(rename = "ingest.imported")]
    IngestImported,
    #[serde(rename = "ingest.skipped")]
    IngestSkipped,
    #[serde(rename = "ingest.duplicate")]
    IngestDuplicate,
    #[serde(rename = "ingest.failed")]
    IngestFailed,
    #[serde(rename = "source.failed")]
    SourceFailed,
    #[serde(rename = "source.recovered")]
    SourceRecovered,
    #[serde(rename = "document.processing.ocr.started")]
    DocumentOcrStarted,
    #[serde(rename = "document.processing.ocr.completed")]
    DocumentOcrCompleted,
    #[serde(rename = "document.processing.ocr.failed")]
    DocumentOcrFailed,
    #[serde(rename = "document.processing.ocr.skipped")]
    DocumentOcrSkipped,
    #[serde(rename = "document.processing.ocr.retry_scheduled")]
    DocumentOcrRetryScheduled,
    #[serde(rename = "document.processing.embedding.started")]
    DocumentEmbeddingStarted,
    #[serde(rename = "document.processing.embedding.completed")]
    DocumentEmbeddingCompleted,
    #[serde(rename = "document.processing.embedding.failed")]
    DocumentEmbeddingFailed,
    #[serde(rename = "document.processing.embedding.skipped")]
    DocumentEmbeddingSkipped,
    #[serde(rename = "document.processing.embedding.retry_scheduled")]
    DocumentEmbeddingRetryScheduled,
    #[serde(rename = "document.processing.thumbnail.started")]
    DocumentThumbnailStarted,
    #[serde(rename = "document.processing.thumbnail.completed")]
    DocumentThumbnailCompleted,
    #[serde(rename = "document.processing.thumbnail.failed")]
    DocumentThumbnailFailed,
    #[serde(rename = "document.processing.thumbnail.skipped")]
    DocumentThumbnailSkipped,
    #[serde(rename = "document.processing.thumbnail.retry_scheduled")]
    DocumentThumbnailRetryScheduled,
    #[serde(rename = "workflow.scheduled")]
    Scheduled,
}

impl WorkflowTrigger {
    pub const ALL: [WorkflowTrigger; 55] = [
        Self::DocumentUploaded,
        Self::DocumentProcessed,
        Self::DocumentUpdated,
        Self::DocumentTitleChanged,
        Self::DocumentTagsChanged,
        Self::DocumentCorrespondentChanged,
        Self::DocumentTypeChanged,
        Self::DocumentNoteCreated,
        Self::DocumentNoteUpdated,
        Self::DocumentNoteDeleted,
        Self::DocumentCommentCreated,
        Self::DocumentCommentUpdated,
        Self::DocumentCommentDeleted,
        Self::SavedViewCreated,
        Self::SavedViewUpdated,
        Self::SavedViewDeleted,
        Self::MetadataFieldCreated,
        Self::MetadataFieldUpdated,
        Self::MetadataFieldDeleted,
        Self::MetadataFieldRestored,
        Self::MetadataOptionCreated,
        Self::MetadataOptionUpdated,
        Self::MetadataOptionDeleted,
        Self::MetadataOptionRestored,
        Self::DocumentMetadataChanged,
        Self::DocumentMovedOut,
        Self::DocumentMovedIn,
        Self::DocumentTrashed,
        Self::DocumentRestored,
        Self::DocumentPurgeRequested,
        Self::DocumentPurged,
        Self::DocumentDeleted,
        Self::IngestReceived,
        Self::IngestImported,
        Self::IngestSkipped,
        Self::IngestDuplicate,
        Self::IngestFailed,
        Self::SourceFailed,
        Self::SourceRecovered,
        Self::DocumentOcrStarted,
        Self::DocumentOcrCompleted,
        Self::DocumentOcrFailed,
        Self::DocumentOcrSkipped,
        Self::DocumentOcrRetryScheduled,
        Self::DocumentEmbeddingStarted,
        Self::DocumentEmbeddingCompleted,
        Self::DocumentEmbeddingFailed,
        Self::DocumentEmbeddingSkipped,
        Self::DocumentEmbeddingRetryScheduled,
        Self::DocumentThumbnailStarted,
        Self::DocumentThumbnailCompleted,
        Self::DocumentThumbnailFailed,
        Self::DocumentThumbnailSkipped,
        Self::DocumentThumbnailRetryScheduled,
        Self::Scheduled,
    ];

    /// The wire name, as used in payloads and the database
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DocumentUploaded => "document.uploaded",
            Self::DocumentProcessed => "document.processed",
            Self::DocumentUpdated => "document.updated",
            Self::DocumentTitleChanged => "document.title_changed",
            Self::DocumentTagsChanged => "document.tags_changed",
            Self::DocumentCorrespondentChanged => "document.correspondent_changed",
            Self::DocumentTypeChanged => "document.document_type_changed",
            Self::DocumentNoteCreated => "document_note.created",
            Self::DocumentNoteUpdated => "document_note.updated",
            Self::DocumentNoteDeleted => "document_note.deleted",
            Self::DocumentCommentCreated => "document_comment.created",
            Self::DocumentCommentUpdated => "document_comment.updated",
            Self::DocumentCommentDeleted => "document_comment.deleted",
            Self::SavedViewCreated => "saved_view.created",
            Self::SavedViewUpdated => "saved_view.updated",
            Self::SavedViewDeleted => "saved_view.deleted",
            Self::MetadataFieldCreated => "metadata_field.created",
            Self::MetadataFieldUpdated => "metadata_field.updated",
            Self::MetadataFieldDeleted => "metadata_field.deleted",
            Self::MetadataFieldRestored => "metadata_field.restored",
            Self::MetadataOptionCreated => "metadata_option.created",
            Self::MetadataOptionUpdated => "metadata_option.updated",
            Self::MetadataOptionDeleted => "metadata_option.deleted",
            Self::MetadataOptionRestored => "metadata_option.restored",
            Self::DocumentMetadataChanged => "document.metadata.changed",
            Self::DocumentMovedOut => "document.moved_out",
            Self::DocumentMovedIn => "document.moved_in",
            Self::DocumentTrashed => "document.trashed",
            Self::DocumentRestored => "document.restored",
            Self::DocumentPurgeRequested => "document.purge_requested",
            Self::DocumentPurged => "document.purged",
            Self::DocumentDeleted => "document.deleted",
            Self::IngestReceived => "ingest.received",
            Self::IngestImported => "ingest.imported",
            Self::IngestSkipped => "ingest.skipped",
            Self::IngestDuplicate => "ingest.duplicate",
            Self::IngestFailed => "ingest.failed",
            Self::SourceFailed => "source.failed",
            Self::SourceRecovered => "source.recovered",
            Self::DocumentOcrStarted => "document.processing.ocr.started",
            Self::DocumentOcrCompleted => "document.processing.ocr.completed",
            Self::DocumentOcrFailed => "document.processing.ocr.failed",
            Self::DocumentOcrSkipped => "document.processing.ocr.skipped",
            Self::DocumentOcrRetryScheduled => "document.processing.ocr.retry_scheduled",
            Self::DocumentEmbeddingStarted => "document.processing.embedding.started",
            Self::DocumentEmbeddingCompleted => "document.processing.embedding.completed",
            Self::DocumentEmbeddingFailed => "document.processing.embedding.failed",
            Self::DocumentEmbeddingSkipped => "document.processing.embedding.skipped",
            Self::DocumentEmbeddingRetryScheduled => {
                "document.processing.embedding.retry_scheduled"
            }
            Self::DocumentThumbnailStarted => "document.processing.thumbnail.started",
            Self::DocumentThumbnailCompleted => "document.processing.thumbnail.completed",
            Self::DocumentThumbnailFailed => "document.processing.thumbnail.failed",
            Self::DocumentThumbnailSkipped => "document.processing.thumbnail.skipped",
            Self::DocumentThumbnailRetryScheduled => {
                "document.processing.thumbnail.retry_scheduled"
            }
            Self::Scheduled => "workflow.scheduled",
        }
    }

    /// How a person would say it: "When a document has been read"
    pub fn label(&self) -> &'static str {
        match self {
            Self::DocumentUploaded => "When a document is added",
            Self::DocumentProcessed => "When a document has been read",
            Self::DocumentUpdated => "When a document's details change",
            Self::DocumentTitleChanged => "When a document title changes",
            Self::DocumentTagsChanged => "When document tags change",
            Self::DocumentCorrespondentChanged => "When a document correspondent changes",
            Self::DocumentTypeChanged => "When a document type changes",
            Self::DocumentNoteCreated => "When a note is created",
            Self::DocumentNoteUpdated => "When a note is updated",
            Self::DocumentNoteDeleted => "When a note is deleted",
            Self::DocumentCommentCreated => "When a comment is created",
            Self::DocumentCommentUpdated => "When a comment is updated",
            Self::DocumentCommentDeleted => "When a comment is deleted",
            Self::SavedViewCreated => "When a shared saved view is created",
            Self::SavedViewUpdated => "When a shared saved view is updated",
            Self::SavedViewDeleted => "When a shared saved view is deleted",
            Self::MetadataFieldCreated => "When a custom field is created",
            Self::MetadataFieldUpdated => "When a custom field is updated",
            Self::MetadataFieldDeleted => "When a custom field is retired",
            Self::MetadataFieldRestored => "When a custom field is restored",
            Self::MetadataOptionCreated => "When a select option is created",
            Self::MetadataOptionUpdated => "When a select option is updated",
            Self::MetadataOptionDeleted => "When a select option is retired",
            Self::MetadataOptionRestored => "When a select option is restored",
            Self::DocumentMetadataChanged => "When a document's custom fields change",
            Self::DocumentMovedOut => "When a document leaves this vault",
            Self::DocumentMovedIn => "When a document enters this vault",
            Self::DocumentTrashed => "When a document is moved to trash",
            Self::DocumentRestored => "When a document is restored",
            Self::DocumentPurgeRequested => "When permanent deletion is requested",
            Self::DocumentPurged => "When a document is permanently deleted",
            Self::DocumentDeleted => "When a document is deleted",
            Self::IngestReceived => "When a file arrives, before import",
            Self::IngestImported => "When an item is imported",
            Self::IngestSkipped => "When a rule skips an item",
            Self::IngestDuplicate => "When a duplicate item is found",
            Self::IngestFailed => "When an item import fails",
            Self::SourceFailed => "When a source becomes unavailable",
            Self::SourceRecovered => "When a source recovers",
            Self::DocumentOcrStarted => "Text extraction started",
            Self::DocumentOcrCompleted => "Text extraction completed",
            Self::DocumentOcrFailed => "Text extraction failed",
            Self::DocumentOcrSkipped => "Text extraction skipped",
            Self::DocumentOcrRetryScheduled => "Text extraction retry scheduled",
            Self::DocumentEmbeddingStarted => "Search indexing started",
            Self::DocumentEmbeddingCompleted => "Search indexing completed",
            Self::DocumentEmbeddingFailed => "Search indexing failed",
            Self::DocumentEmbeddingSkipped => "Search indexing skipped",
            Self::DocumentEmbeddingRetryScheduled => "Search indexing retry scheduled",
            Self::DocumentThumbnailStarted => "Preview generation started",
            Self::DocumentThumbnailCompleted => "Preview generation completed",
            Self::DocumentThumbnailFailed => "Preview generation failed",
            Self::DocumentThumbnailSkipped => "Preview generation skipped",
            Self::DocumentThumbnailRetryScheduled => "Preview generation retry scheduled",
            Self::Scheduled => "On a schedule",
        }
    }

    pub fn target(&self) -> WorkflowTarget {
        match self {
            Self::DocumentUploaded
            | Self::DocumentProcessed
            | Self::DocumentUpdated
            | Self::DocumentTitleChanged
            | Self::DocumentTagsChanged
            | Self::DocumentCorrespondentChanged
            | Self::DocumentTypeChanged
            | Self::DocumentNoteCreated
            | Self::DocumentNoteUpdated
            | Self::DocumentNoteDeleted
            | Self::DocumentCommentCreated
            | Self::DocumentCommentUpdated
            | Self::DocumentCommentDeleted
            | Self::DocumentMetadataChanged
            | Self::DocumentMovedIn
            | Self::DocumentRestored
            | Self::DocumentOcrStarted
            | Self::DocumentOcrCompleted
            | Self::DocumentOcrFailed
            | Self::DocumentOcrSkipped
            | Self::DocumentOcrRetryScheduled
            | Self::DocumentEmbeddingStarted
            | Self::DocumentEmbeddingCompleted
            | Self::DocumentEmbeddingFailed
            | Self::DocumentEmbeddingSkipped
            | Self::DocumentEmbeddingRetryScheduled
            | Self::DocumentThumbnailStarted
            | Self::DocumentThumbnailCompleted
            | Self::DocumentThumbnailFailed
            | Self::DocumentThumbnailSkipped
            | Self::DocumentThumbnailRetryScheduled => WorkflowTarget::Document,
            Self::SavedViewCreated
            | Self::SavedViewUpdated
            | Self::SavedViewDeleted
            | Self::MetadataFieldCreated
            | Self::MetadataFieldUpdated
            | Self::MetadataFieldDeleted
            | Self::MetadataFieldRestored
            | Self::MetadataOptionCreated
            | Self::MetadataOptionUpdated
            | Self::MetadataOptionDeleted
            | Self::MetadataOptionRestored
            | Self::DocumentMovedOut
            | Self::DocumentTrashed
            | Self::DocumentPurgeRequested
            | Self::DocumentPurged
            | Self::DocumentDeleted
            | Self::IngestImported
            | Self::IngestSkipped
            | Self::IngestDuplicate
            | Self::IngestFailed
            | Self::SourceFailed
            | Self::SourceRecovered
            | Self::Scheduled => WorkflowTarget::Event,
            Self::IngestReceived => WorkflowTarget::IngestCandidate,
        }
    }

    pub fn is_processing_outcome(self) -> bool {
        self.as_str().starts_with("document.processing.")
    }

    pub fn is_ingest_outcome(self) -> bool {
        matches!(
            self,
            Self::IngestImported
                | Self::IngestSkipped
                | Self::IngestDuplicate
                | Self::IngestFailed
                | Self::SourceFailed
                | Self::SourceRecovered
        )
    }

    pub fn is_custom_metadata(self) -> bool {
        matches!(
            self,
            Self::MetadataFieldCreated
                | Self::MetadataFieldUpdated
                | Self::MetadataFieldDeleted
                | Self::MetadataFieldRestored
                | Self::MetadataOptionCreated
                | Self::MetadataOptionUpdated
                | Self::MetadataOptionDeleted
                | Self::MetadataOptionRestored
                | Self::DocumentMetadataChanged
        )
    }

    pub fn is_saved_view(self) -> bool {
        matches!(
            self,
            Self::SavedViewCreated | Self::SavedViewUpdated | Self::SavedViewDeleted
        )
    }

    pub fn entry_transition(self) -> Option<(super::EntryKind, &'static str)> {
        match self {
            Self::DocumentNoteCreated => Some((super::EntryKind::Note, "created")),
            Self::DocumentNoteUpdated => Some((super::EntryKind::Note, "updated")),
            Self::DocumentNoteDeleted => Some((super::EntryKind::Note, "deleted")),
            Self::DocumentCommentCreated => Some((super::EntryKind::Comment, "created")),
            Self::DocumentCommentUpdated => Some((super::EntryKind::Comment, "updated")),
            Self::DocumentCommentDeleted => Some((super::EntryKind::Comment, "deleted")),
            _ => None,
        }
    }

    /// Parse the wire name
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == value)
    }

    /// Whether a workflow should fire at most once per document for this trigger
    ///
    /// Processing can be re-run (repair, reprocess), but lifecycle milestones
    /// like "uploaded" and "processed" must not notify the same document twice.
    pub fn once_per_document(&self) -> bool {
        matches!(self, Self::DocumentUploaded | Self::DocumentProcessed)
    }
}

/// Conditions a document must meet; every set condition has to match
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowConditions {
    /// MIME type, exact or with a wildcard subtype such as `image/*`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,

    /// The document carries this tag
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub tag_id: Option<Id>,

    /// The document has this correspondent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub correspondent_id: Option<Id>,

    /// The document has this document type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub document_type_id: Option<Id>,

    /// Glob (`*` and `?`) matched case-insensitively against the original filename
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename_glob: Option<String>,

    /// Where the document came from: folder path, email sender, subject, and so on
    #[serde(default, skip_serializing_if = "SourceConditions::is_empty")]
    pub source: SourceConditions,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WorkflowEventConditions, required = false))]
    pub event: Option<WorkflowEventConditions>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "openapi", schema(no_recursion))]
    pub all: Vec<WorkflowConditions>,

    /// Omitted means unconstrained; an empty list never matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(no_recursion))]
    pub any: Option<Vec<WorkflowConditions>>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "openapi", schema(no_recursion))]
    pub none: Vec<WorkflowConditions>,
}

impl WorkflowConditions {
    /// Reject invalid source patterns and trees exceeding eight levels or 64 nodes.
    pub fn validate(&self) -> Result<()> {
        let mut pending = vec![(self, 0)];
        let mut count = 0;
        while let Some((conditions, depth)) = pending.pop() {
            count += 1;
            if depth > 8 || count > 64 {
                return Err(Error::Validation(
                    "Workflow conditions allow at most eight nested groups and 64 condition objects"
                        .into(),
                ));
            }
            conditions.source.validate()?;
            if let Some(event) = &conditions.event {
                event.validate()?;
            }
            for child in conditions
                .all
                .iter()
                .chain(conditions.any.iter().flatten())
                .chain(&conditions.none)
            {
                pending.push((child, depth + 1));
            }
        }
        Ok(())
    }

    /// Whether the document satisfies every set condition
    pub fn matches(&self, document: &Document, tag_ids: &[Id]) -> bool {
        self.matches_metadata(
            document.mime_type.as_deref(),
            document.original_filename.as_deref(),
            Some(&document.source),
            tag_ids,
            document.correspondent_id,
            document.document_type_id,
        )
    }

    /// Whether the event-time metadata satisfies every set condition.
    pub fn matches_snapshot(&self, snapshot: &crate::EventSnapshot) -> bool {
        let tag_ids: Vec<_> = snapshot.tags.iter().map(|tag| tag.id).collect();
        self.matches_metadata(
            snapshot.mime_type.as_deref(),
            snapshot.original_filename.as_deref(),
            snapshot.source.as_ref(),
            &tag_ids,
            snapshot.correspondent_id,
            snapshot.document_type_id,
        )
    }

    /// Matches one occurrence without substituting current metadata for missing snapshots.
    pub fn matches_occurrence(
        &self,
        snapshot: Option<&crate::EventSnapshot>,
        event: &crate::DomainEventEnvelope,
    ) -> bool {
        if snapshot.is_some_and(|snapshot| {
            snapshot.vault_id != event.vault_id()
                || Some(snapshot.document_id) != event.document_id()
        }) {
            return false;
        }
        let tag_ids: Vec<_> = snapshot
            .map(|snapshot| snapshot.tags.iter().map(|tag| tag.id).collect())
            .unwrap_or_default();
        self.matches_occurrence_parts(snapshot, event, &tag_ids)
    }

    fn matches_occurrence_parts(
        &self,
        snapshot: Option<&crate::EventSnapshot>,
        event: &crate::DomainEventEnvelope,
        tag_ids: &[Id],
    ) -> bool {
        self.event
            .as_ref()
            .is_none_or(|condition| condition.matches(event))
            && match snapshot {
                Some(snapshot) => self.matches_metadata_leaf(
                    snapshot.mime_type.as_deref(),
                    snapshot.original_filename.as_deref(),
                    snapshot.source.as_ref(),
                    tag_ids,
                    snapshot.correspondent_id,
                    snapshot.document_type_id,
                ),
                None => !self.has_snapshot_filters(),
            }
            && self.matches_groups(|child| child.matches_occurrence_parts(snapshot, event, tag_ids))
    }

    fn matches_metadata(
        &self,
        mime: Option<&str>,
        filename: Option<&str>,
        source: Option<&DocumentSource>,
        tag_ids: &[Id],
        correspondent_id: Option<Id>,
        document_type_id: Option<Id>,
    ) -> bool {
        self.event.is_none()
            && self.matches_metadata_leaf(
                mime,
                filename,
                source,
                tag_ids,
                correspondent_id,
                document_type_id,
            )
            && self.matches_groups(|child| {
                child.matches_metadata(
                    mime,
                    filename,
                    source,
                    tag_ids,
                    correspondent_id,
                    document_type_id,
                )
            })
    }

    fn matches_metadata_leaf(
        &self,
        mime: Option<&str>,
        filename: Option<&str>,
        source: Option<&DocumentSource>,
        tag_ids: &[Id],
        correspondent_id: Option<Id>,
        document_type_id: Option<Id>,
    ) -> bool {
        self.matches_file(mime, filename, source)
            && self.tag_id.is_none_or(|id| tag_ids.contains(&id))
            && self
                .correspondent_id
                .is_none_or(|id| correspondent_id == Some(id))
            && self
                .document_type_id
                .is_none_or(|id| document_type_id == Some(id))
    }

    fn has_snapshot_filters(&self) -> bool {
        self.mime_type.is_some()
            || self.filename_glob.is_some()
            || self.tag_id.is_some()
            || self.correspondent_id.is_some()
            || self.document_type_id.is_some()
            || !self.source.is_empty()
    }

    fn validate_event_conditions(&self, trigger: WorkflowTrigger) -> Result<()> {
        if trigger.is_saved_view() && self.has_snapshot_filters() {
            return Err(Error::Validation(
                "Saved view workflows do not have document filters".into(),
            ));
        }
        if let Some(event) = &self.event {
            if let Some(ingest) = &event.ingest {
                ingest.validate_trigger(trigger)?;
            }
            if let Some(metadata) = &event.metadata {
                metadata.validate_trigger(trigger)?;
            }
            if let Some(processing) = &event.processing {
                processing.validate_trigger(trigger)?;
            }
        }
        for child in self
            .all
            .iter()
            .chain(self.any.iter().flatten())
            .chain(&self.none)
        {
            child.validate_event_conditions(trigger)?;
        }
        Ok(())
    }

    fn requires_event_context(&self) -> bool {
        self.event.is_some()
            || self
                .all
                .iter()
                .chain(self.any.iter().flatten())
                .chain(&self.none)
                .any(Self::requires_event_context)
    }

    fn matches_groups(&self, matches: impl Fn(&Self) -> bool) -> bool {
        self.all.iter().all(&matches)
            && self
                .any
                .as_ref()
                .is_none_or(|group| group.iter().any(&matches))
            && !self.none.iter().any(matches)
    }

    /// Whether a file that is not imported yet satisfies the conditions
    ///
    /// Stored metadata predicates evaluate false because a candidate has no
    /// tag, correspondent, or type; group operators retain their usual semantics.
    pub fn matches_candidate(
        &self,
        mime_type: &str,
        filename: &str,
        source: &DocumentSource,
    ) -> bool {
        self.matches_metadata(
            Some(mime_type),
            Some(filename),
            Some(source),
            &[],
            None,
            None,
        )
    }

    fn matches_file(
        &self,
        mime_type: Option<&str>,
        filename: Option<&str>,
        source: Option<&DocumentSource>,
    ) -> bool {
        if let Some(pattern) = &self.mime_type {
            if !mime_type.is_some_and(|mime| glob_match(pattern, mime)) {
                return false;
            }
        }
        if let Some(pattern) = &self.filename_glob {
            if !filename.is_some_and(|name| glob_match(pattern, name)) {
                return false;
            }
        }
        source.map_or_else(
            || self.source.is_empty(),
            |source| self.source.matches(source),
        )
    }
}

/// Case-insensitive glob where `*` matches any run and `?` any single character
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();

    let (mut p, mut t) = (0, 0);
    let mut backtrack: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            backtrack = Some((p, t));
            p += 1;
        } else if let Some((star_p, star_t)) = backtrack {
            p = star_p + 1;
            t = star_t + 1;
            backtrack = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// HTTP method used for webhook delivery
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "UPPERCASE")]
pub enum WebhookMethod {
    #[default]
    Post,
}

/// A step a workflow performs, in order
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowAction {
    /// Send the event payload to an HTTP endpoint
    Webhook {
        /// Destination; only `http` and `https` are accepted
        url: String,
        #[serde(default)]
        method: WebhookMethod,
        /// Extra request headers; `X-Folio-*` headers cannot be overridden
        #[serde(default)]
        headers: BTreeMap<String, String>,
        /// Shared secret used to sign deliveries; unsigned when absent
        #[serde(default, skip_serializing_if = "Option::is_none")]
        secret: Option<String>,
    },
    /// Attach tags to the document
    AddTags {
        #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
        tag_ids: Vec<Id>,
    },
    /// Set the document's correspondent
    SetCorrespondent {
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
        correspondent_id: Id,
    },
    /// Set the document's type
    SetDocumentType {
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
        document_type_id: Id,
    },
    /// Discard the file instead of importing it; only valid for `ingest.received`
    SkipImport,
    /// Set the document's title.
    RenameDocument { title: String },
    /// Remove tags from the document.
    RemoveTags {
        #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
        tag_ids: Vec<Id>,
    },
    /// Remove the correspondence assignment.
    ClearCorrespondent,
    /// Remove the document type assignment.
    ClearDocumentType,
    /// Move the document to Trash as the final action.
    TrashDocument,
    /// Set the document's date without a time or time zone.
    SetDocumentDate { date: DocumentDate },
    /// Clear the document's date.
    ClearDocumentDate,
    /// Mark a document reviewed or unreviewed as a system decision.
    SetReviewed { reviewed: bool },
    SetMetadata {
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
        field_id: Id,
        value: MetadataValue,
    },
    ClearMetadata {
        #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
        field_id: Id,
    },
}

impl WorkflowAction {
    pub fn validate_document_title(title: &str) -> Result<()> {
        if title.trim().is_empty() {
            return Err(Error::Validation("Enter a document title".into()));
        }
        if title.contains('\0') {
            return Err(Error::Validation(
                "Document title cannot contain a NUL character".into(),
            ));
        }
        Ok(())
    }

    /// Whether the action sends the event payload rather than changing the document
    pub fn is_webhook(&self) -> bool {
        matches!(self, Self::Webhook { .. })
    }
}

/// A configured automation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Workflow {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    pub name: String,
    pub enabled: bool,
    /// First selected trigger, retained for existing clients.
    pub trigger: WorkflowTrigger,
    /// Selected triggers in the order saved by the owner.
    #[serde(default)]
    pub triggers: Vec<WorkflowTrigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WorkflowCadence, required = false))]
    pub cadence: Option<WorkflowCadence>,
    pub conditions: WorkflowConditions,
    pub actions: Vec<WorkflowAction>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Revision to send when changing the trigger.
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(required = false))]
    pub revision: i64,
    #[serde(default)]
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub created_by: Option<UserId>,
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub updated_by: Option<UserId>,
}

impl Workflow {
    pub fn check_revision(&self, expected: Option<i64>) -> Result<()> {
        if expected.is_some_and(|revision| revision != self.revision) {
            return Err(Error::Conflict(
                "Workflow changed. Reload it before editing.".into(),
            ));
        }
        Ok(())
    }
}

/// Fields for creating or replacing a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(try_from = "WorkflowInputWire")]
pub struct WorkflowInput {
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Legacy selection, or the first trigger when both fields are supplied.
    #[cfg_attr(feature = "openapi", schema(required = false))]
    pub trigger: WorkflowTrigger,
    /// Ordered nonempty trigger list. Omit for a legacy single-trigger request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<WorkflowTrigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = WorkflowCadence, required = false))]
    pub cadence: Option<WorkflowCadence>,
    #[serde(default)]
    pub conditions: WorkflowConditions,
    pub actions: Vec<WorkflowAction>,
}

#[derive(Deserialize)]
struct WorkflowInputWire {
    name: String,
    #[serde(default = "default_enabled")]
    enabled: bool,
    trigger: Option<WorkflowTrigger>,
    triggers: Option<Vec<WorkflowTrigger>>,
    cadence: Option<WorkflowCadence>,
    #[serde(default)]
    conditions: WorkflowConditions,
    actions: Vec<WorkflowAction>,
}

impl TryFrom<WorkflowInputWire> for WorkflowInput {
    type Error = Error;

    fn try_from(wire: WorkflowInputWire) -> Result<Self> {
        if wire.triggers.as_ref().is_some_and(Vec::is_empty) {
            return Err(Error::Validation(
                "Select at least one workflow trigger".into(),
            ));
        }
        let triggers = wire.triggers.unwrap_or_default();
        let trigger = triggers
            .first()
            .copied()
            .or(wire.trigger)
            .ok_or_else(|| Error::Validation("Select at least one workflow trigger".into()))?;
        if wire.trigger.is_some_and(|legacy| legacy != trigger) {
            return Err(Error::Validation(
                "The legacy workflow trigger must match the first selection".into(),
            ));
        }
        Ok(Self {
            name: wire.name,
            enabled: wire.enabled,
            trigger,
            triggers,
            cadence: wire.cadence,
            conditions: wire.conditions,
            actions: wire.actions,
        })
    }
}

fn default_enabled() -> bool {
    true
}

impl WorkflowInput {
    pub fn selected_triggers(&self) -> &[WorkflowTrigger] {
        if self.triggers.is_empty() {
            std::slice::from_ref(&self.trigger)
        } else {
            &self.triggers
        }
    }

    pub fn keep_legacy_trigger_selection(&mut self, existing: &Workflow) {
        if self.triggers.is_empty() && existing.trigger == self.trigger {
            self.triggers.clone_from(&existing.triggers);
        }
    }

    /// Reject workflows that could never run correctly
    ///
    /// Webhook URLs must be `http` or `https`. Private and loopback addresses
    /// are allowed on purpose: Folio is self-hosted and its automation targets
    /// (Home Assistant, n8n) usually live on the same LAN.
    pub fn validate(&self) -> Result<()> {
        let invalid = |msg: &str| Err(Error::Validation(msg.to_string()));

        if self.name.trim().is_empty() {
            return invalid("Workflow name must not be empty");
        }
        let triggers = self.selected_triggers();
        if triggers.first() != Some(&self.trigger) {
            return invalid("The legacy workflow trigger must match the first selection");
        }
        if triggers.len() > 1 && triggers.contains(&WorkflowTrigger::Scheduled) {
            return invalid("A clock schedule must be the only trigger");
        }
        match (self.trigger, self.cadence) {
            (WorkflowTrigger::Scheduled, Some(cadence)) => {
                cadence.validate()?;
                if self.conditions != WorkflowConditions::default() {
                    return invalid("A clock schedule has no document or event to filter");
                }
            }
            (WorkflowTrigger::Scheduled, None) => return invalid("Set a schedule"),
            (_, Some(_)) => return invalid("A cadence requires a scheduled trigger"),
            (_, None) => {}
        }
        if self.actions.is_empty() {
            return invalid("A workflow needs at least one action");
        }
        for (index, action) in self.actions.iter().enumerate() {
            match action {
                WorkflowAction::Webhook { url, headers, .. } => {
                    validate_webhook_url(url)?;
                    if headers.keys().any(|k| k.trim().is_empty()) {
                        return invalid("Webhook header names must not be empty");
                    }
                }
                WorkflowAction::AddTags { tag_ids } if tag_ids.is_empty() => {
                    return invalid("add_tags needs at least one tag");
                }
                WorkflowAction::RemoveTags { tag_ids } if tag_ids.is_empty() => {
                    return invalid("remove_tags needs at least one tag");
                }
                WorkflowAction::RenameDocument { title } => {
                    WorkflowAction::validate_document_title(title)?;
                }
                WorkflowAction::SetMetadata { field_id, .. }
                | WorkflowAction::ClearMetadata { field_id }
                    if field_id.is_nil() =>
                {
                    return invalid("Select a custom field");
                }
                WorkflowAction::SetMetadata { value, .. } => {
                    value.clone().validate(value.kind(), false)?;
                }
                WorkflowAction::TrashDocument if index + 1 != self.actions.len() => {
                    return invalid("Move to Trash must be the final action");
                }
                _ => {}
            }
            if !self.trigger.target().accepts(action) {
                return invalid(if self.trigger == WorkflowTrigger::DocumentDeleted {
                    "Only webhook actions can run when a document is deleted"
                } else if self.trigger == WorkflowTrigger::Scheduled {
                    "Scheduled workflows take only webhook actions"
                } else if self.trigger.is_ingest_outcome() {
                    "Ingestion outcome workflows take only webhook actions"
                } else if self.trigger.is_custom_metadata()
                    && self.trigger.target() == WorkflowTarget::Event
                {
                    "Custom field and option workflows take only webhook actions"
                } else if self.trigger.is_saved_view() {
                    "Saved view workflows take only webhook actions"
                } else if self.trigger.target() == WorkflowTarget::Event {
                    "Only webhook actions can run for this document lifecycle event"
                } else {
                    "ingest.received workflows take only skip_import, and skip_import needs that trigger"
                });
            }
        }
        let selected: Vec<_> = triggers.iter().map(|trigger| trigger.as_str()).collect();
        WorkflowTriggerRegistry::built_in().validate_actions(&selected, &self.actions)?;
        self.conditions.validate()?;
        for trigger in triggers {
            self.conditions.validate_event_conditions(*trigger)?;
            if *trigger == WorkflowTrigger::IngestReceived
                && self.conditions.requires_event_context()
            {
                return invalid(
                    "Pre-import workflows do not have a committed event actor or subject",
                );
            }
        }
        Ok(())
    }
}

/// The URL with any `user:password@` part masked, safe to show in messages and logs
pub fn redact_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(mut parsed) => {
            if !parsed.username().is_empty() || parsed.password().is_some() {
                let _ = parsed.set_username("***");
                let _ = parsed.set_password(None);
            }
            parsed.to_string()
        }
        Err(_) => "<invalid url>".to_string(),
    }
}

/// Accept only absolute `http`/`https` URLs with a host
pub fn validate_webhook_url(raw: &str) -> Result<()> {
    let parsed =
        url::Url::parse(raw).map_err(|e| Error::Validation(format!("Invalid webhook URL: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(Error::Validation(format!(
            "Webhook URL must use http or https, not '{}'",
            parsed.scheme()
        )));
    }
    if parsed.host_str().is_none() {
        return Err(Error::Validation("Webhook URL needs a host".to_string()));
    }
    Ok(())
}

/// Where a run is in its lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// Waiting for a first attempt or a retry
    Pending,
    Succeeded,
    /// Gave up, either on a permanent error or after the last attempt
    Failed,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// One execution of a workflow for one event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkflowRun {
    /// Sent as `X-Folio-Run`; deliveries have separate per-action IDs.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub workflow_id: Id,
    pub event: WorkflowTrigger,
    /// Not a foreign key: runs outlive the documents they were about
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub document_id: Option<Id>,
    pub status: RunStatus,
    pub attempts: u32,
    /// How many actions already finished, so a retry resumes where it stopped
    pub completed_actions: u32,
    pub last_error: Option<String>,
    /// HTTP status of the latest webhook response
    pub response_status: Option<u16>,
    /// The event payload as sent, frozen when the event happened
    #[serde(skip)]
    pub payload: String,
    /// The workflow's actions as they were when the run was created
    ///
    /// Executed instead of the workflow's current list, so editing a workflow
    /// never changes what an already queued run does. May hold webhook secrets,
    /// so it is never serialized.
    #[serde(skip)]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub actions: Vec<WorkflowAction>,
    /// Outbox event that caused the run; at most one run per workflow and event
    #[serde(skip)]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub source_event_id: Option<i64>,
    #[serde(skip)]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub definition_revision: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl WorkflowRun {
    pub fn source_identity(&self) -> Result<Option<WorkflowEventIdentity>> {
        Self::source_identity_from_payload(&self.payload)
    }

    pub fn source_identity_from_payload(payload: &str) -> Result<Option<WorkflowEventIdentity>> {
        #[derive(Deserialize)]
        struct Context {
            #[serde(default)]
            source_event: Option<WorkflowEventIdentity>,
        }
        serde_json::from_str::<Context>(payload)
            .map(|context| context.source_event)
            .map_err(|error| {
                Error::Repository(format!("Invalid captured workflow payload: {error}"))
            })
    }

    pub fn new(
        vault_id: VaultId,
        workflow_id: Id,
        event: WorkflowTrigger,
        document_id: Option<Id>,
        payload: String,
    ) -> Self {
        Self {
            id: Id::new_v4(),
            vault_id,
            workflow_id,
            event,
            document_id,
            status: RunStatus::Pending,
            attempts: 0,
            completed_actions: 0,
            last_error: None,
            response_status: None,
            payload,
            actions: Vec::new(),
            source_event_id: None,
            definition_revision: None,
            created_at: Utc::now(),
            finished_at: None,
        }
    }

    pub fn with_definition(mut self, workflow: &Workflow) -> Self {
        self.definition_revision = Some(workflow.revision);
        self.with_actions(workflow.actions.clone())
    }

    /// Freeze the actions to execute
    pub fn with_actions(mut self, actions: Vec<WorkflowAction>) -> Self {
        self.actions = actions;
        self
    }

    /// Record the outbox event that caused this run
    pub fn with_source_event(mut self, event_id: i64) -> Self {
        self.source_event_id = Some(event_id);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::VaultId;

    fn doc(filename: &str, mime: &str) -> Document {
        Document::new(VaultId::new(), "t")
            .with_filename(filename)
            .with_mime_type(mime)
    }

    fn note_event(actor: Option<&str>) -> crate::DomainEventEnvelope {
        let event = crate::DomainEventEnvelope::new(
            "document_note.created",
            VaultId::new(),
            crate::DomainEventSubject {
                subject_type: "document_note".into(),
                id: Id::from_u128(1),
            },
            Some(Id::new_v4()),
            crate::EventAudience::Vault,
            serde_json::json!({"entry_kind": "note", "transition": "created"}),
        )
        .unwrap();
        let mut encoded = serde_json::to_value(event).unwrap();
        encoded["actor_id"] = serde_json::json!(actor);
        serde_json::from_value(encoded).unwrap()
    }

    #[test]
    fn event_conditions_match_only_the_selected_user_and_subject() {
        let event = note_event(Some("editor"));
        let conditions = WorkflowConditions {
            event: Some(WorkflowEventConditions {
                subject_id: Some(event.subject().id),
                actor: Some(crate::models::WorkflowActorCondition::User {
                    user_id: "editor".into(),
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(conditions.matches_occurrence(None, &event));
        assert!(!conditions.matches_occurrence(None, &note_event(Some("other"))));
        assert!(!conditions.matches_occurrence(None, &note_event(None)));
    }

    #[test]
    fn event_conditions_reject_a_different_subject() {
        let conditions = WorkflowConditions {
            event: Some(WorkflowEventConditions {
                subject_id: Some(Id::from_u128(2)),
                actor: None,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!conditions.matches_occurrence(None, &note_event(Some("editor"))));
    }

    #[test]
    fn pre_import_conditions_reject_nested_committed_event_filters() {
        let mut workflow = input(
            WorkflowTrigger::IngestReceived,
            vec![WorkflowAction::SkipImport],
        );
        workflow.conditions.all = vec![WorkflowConditions {
            event: Some(WorkflowEventConditions::default()),
            ..Default::default()
        }];
        assert!(workflow.validate().is_err());
    }

    #[test]
    fn system_actor_conditions_require_an_explicit_null_actor() {
        let conditions = WorkflowConditions {
            event: Some(WorkflowEventConditions {
                subject_id: None,
                actor: Some(crate::models::WorkflowActorCondition::System),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(conditions.matches_occurrence(None, &note_event(None)));
        assert!(!conditions.matches_occurrence(None, &note_event(Some("editor"))));
    }

    #[test]
    fn event_conditions_require_the_same_any_branch_to_match() {
        let event = note_event(Some("editor"));
        let mut document = doc("invoice.pdf", "application/pdf");
        document.id = event.document_id().unwrap();
        document.vault_id = event.vault_id();
        let snapshot = crate::EventSnapshot::from(&crate::events::DeletedDocument {
            document,
            tags: vec![],
        });
        let conditions = WorkflowConditions {
            any: Some(vec![
                WorkflowConditions {
                    mime_type: Some("application/pdf".into()),
                    event: Some(WorkflowEventConditions {
                        subject_id: None,
                        actor: Some(crate::models::WorkflowActorCondition::System),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                WorkflowConditions {
                    mime_type: Some("image/*".into()),
                    event: Some(WorkflowEventConditions {
                        subject_id: Some(event.subject().id),
                        actor: None,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        };
        assert!(!conditions.matches_occurrence(Some(&snapshot), &event));
        assert!(!conditions.matches_occurrence(None, &event));
    }

    #[test]
    fn event_conditions_reject_unavailable_body_filters() {
        assert!(
            serde_json::from_value::<WorkflowEventConditions>(serde_json::json!({
                "body": "secret"
            }))
            .is_err()
        );
    }

    #[test]
    fn missing_event_context_does_not_satisfy_event_conditions() {
        let conditions = WorkflowConditions {
            event: Some(WorkflowEventConditions {
                subject_id: None,
                actor: Some(crate::models::WorkflowActorCondition::System),
                ..Default::default()
            }),
            ..Default::default()
        };
        let document = Document::new(VaultId::new(), "invoice");
        assert!(!conditions.matches(&document, &[]));
    }

    #[test]
    fn nested_event_actor_ids_are_validated() {
        let conditions = WorkflowConditions {
            none: vec![WorkflowConditions {
                event: Some(WorkflowEventConditions {
                    subject_id: None,
                    actor: Some(crate::models::WorkflowActorCondition::User {
                        user_id: "  ".into(),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(conditions.validate().is_err());
    }

    #[test]
    fn metadata_transition_triggers_are_selectable() {
        assert!(WorkflowTrigger::parse("document.title_changed").is_some());
        assert!(WorkflowTrigger::parse("document.tags_changed").is_some());
        assert!(WorkflowTrigger::parse("document.correspondent_changed").is_some());
        assert!(WorkflowTrigger::parse("document.document_type_changed").is_some());
    }

    #[test]
    fn glob_matches_wildcards_case_insensitively() {
        assert!(glob_match("*.pdf", "Invoice.PDF"));
        assert!(glob_match("inv-???.pdf", "inv-042.pdf"));
        assert!(glob_match("image/*", "image/png"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a*b*c", "aXXbYYc"));
        assert!(!glob_match("*.pdf", "invoice.pdf.txt"));
        assert!(!glob_match("inv-???.pdf", "inv-42.pdf"));
        assert!(!glob_match("image/*", "application/pdf"));
    }

    #[test]
    fn empty_conditions_match_everything() {
        assert!(WorkflowConditions::default().matches(&Document::new(VaultId::new(), "t"), &[]));
    }

    #[test]
    fn any_group_requires_one_matching_branch() {
        let conditions: WorkflowConditions = serde_json::from_value(serde_json::json!({
            "any": [{"mime_type": "image/*"}, {"filename_glob": "invoice-*"}]
        }))
        .unwrap();
        assert!(conditions.matches(&doc("scan.png", "image/png"), &[]));
        assert!(conditions.matches(&doc("invoice-42.pdf", "application/pdf"), &[]));
        assert!(!conditions.matches(&doc("letter.pdf", "application/pdf"), &[]));
    }

    #[test]
    fn none_group_excludes_matching_documents() {
        let conditions: WorkflowConditions = serde_json::from_value(serde_json::json!({
            "mime_type": "application/pdf",
            "none": [{"filename_glob": "draft-*"}]
        }))
        .unwrap();
        assert!(conditions.matches(&doc("invoice.pdf", "application/pdf"), &[]));
        assert!(!conditions.matches(&doc("draft-invoice.pdf", "application/pdf"), &[]));
        assert!(!conditions.matches(&doc("invoice.png", "image/png"), &[]));
    }

    #[test]
    fn nested_all_group_requires_every_branch() {
        let conditions: WorkflowConditions = serde_json::from_value(serde_json::json!({
            "all": [{"mime_type": "application/pdf"}, {
                "any": [{"filename_glob": "invoice-*"}, {"filename_glob": "receipt-*"}]
            }]
        }))
        .unwrap();
        assert!(conditions.matches(&doc("receipt-42.pdf", "application/pdf"), &[]));
        assert!(!conditions.matches(&doc("letter.pdf", "application/pdf"), &[]));
        assert!(!conditions.matches(&doc("invoice-42.png", "image/png"), &[]));
    }

    #[test]
    fn empty_any_group_never_matches() {
        let conditions: WorkflowConditions =
            serde_json::from_value(serde_json::json!({"any": []})).unwrap();
        assert!(!conditions.matches(&doc("invoice.pdf", "application/pdf"), &[]));
    }

    #[test]
    fn condition_groups_apply_before_import() {
        let conditions: WorkflowConditions = serde_json::from_value(serde_json::json!({
            "all": [{"mime_type": "application/pdf"}],
            "none": [{"filename_glob": "keep-*"}]
        }))
        .unwrap();
        let source = DocumentSource::default();
        assert!(conditions.matches_candidate("application/pdf", "discard.pdf", &source));
        assert!(!conditions.matches_candidate("application/pdf", "keep-invoice.pdf", &source));
        assert!(!conditions.matches_candidate("image/png", "discard.png", &source));
    }

    #[test]
    fn validation_rejects_excessive_condition_nesting() {
        let mut nested = serde_json::json!({"mime_type": "application/pdf"});
        for _ in 0..9 {
            nested = serde_json::json!({"all": [nested]});
        }
        let mut workflow = input(
            WorkflowTrigger::DocumentUploaded,
            vec![webhook("https://example.com")],
        );
        workflow.conditions = serde_json::from_value(nested).unwrap();
        assert!(workflow.validate().is_err());
    }

    #[test]
    fn validation_rejects_oversized_condition_groups() {
        let mut workflow = input(
            WorkflowTrigger::DocumentUploaded,
            vec![webhook("https://example.com")],
        );
        workflow.conditions = serde_json::from_value(serde_json::json!({
            "any": vec![serde_json::json!({"mime_type": "image/*"}); 65]
        }))
        .unwrap();
        assert!(workflow.validate().is_err());
    }

    #[test]
    fn validation_checks_source_patterns_inside_groups() {
        let mut workflow = input(
            WorkflowTrigger::DocumentUploaded,
            vec![webhook("https://example.com")],
        );
        workflow.conditions = serde_json::from_value(serde_json::json!({
            "none": [{"source": {"email_from": {"regex": "["}}}]
        }))
        .unwrap();
        assert!(workflow.validate().is_err());
    }

    #[test]
    fn groups_match_historical_snapshot_metadata() {
        let conditions: WorkflowConditions = serde_json::from_value(serde_json::json!({
            "any": [{"filename_glob": "invoice-*"}],
            "none": [{"mime_type": "image/*"}]
        }))
        .unwrap();
        let mut document = doc("invoice-42.pdf", "application/pdf");
        let snapshot = crate::EventSnapshot::from(&crate::events::DeletedDocument {
            document: document.clone(),
            tags: vec![],
        });
        document.original_filename = Some("letter.pdf".into());
        assert!(conditions.matches_snapshot(&snapshot));
        assert!(!conditions.matches(&document, &[]));
    }

    #[test]
    fn legacy_conditions_round_trip_without_group_fields() {
        let legacy = serde_json::json!({"mime_type": "application/pdf"});
        let conditions: WorkflowConditions = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(conditions).unwrap(), legacy);
    }

    #[cfg(feature = "openapi")]
    #[test]
    fn condition_group_schema_uses_recursive_references() {
        use utoipa::OpenApi;

        #[derive(OpenApi)]
        #[openapi(components(schemas(WorkflowConditions)))]
        struct Api;

        let schema = serde_json::to_value(Api::openapi()).unwrap();
        let conditions = &schema["components"]["schemas"]["WorkflowConditions"];
        assert_eq!(
            conditions["properties"]["all"]["items"]["$ref"],
            "#/components/schemas/WorkflowConditions"
        );
        assert_eq!(
            conditions["properties"]["none"]["items"]["$ref"],
            "#/components/schemas/WorkflowConditions"
        );
    }

    #[test]
    fn every_set_condition_must_match() {
        let tag = Id::new_v4();
        let sender = Id::new_v4();
        let document = doc("scan.pdf", "application/pdf").with_correspondent(sender);

        let all = WorkflowConditions {
            mime_type: Some("application/pdf".into()),
            tag_id: Some(tag),
            correspondent_id: Some(sender),
            document_type_id: None,
            filename_glob: Some("*.pdf".into()),
            source: Default::default(),
            ..Default::default()
        };
        assert!(all.matches(&document, &[tag]));
        assert!(!all.matches(&document, &[]), "missing tag");

        let wrong_mime = WorkflowConditions {
            mime_type: Some("image/*".into()),
            ..Default::default()
        };
        assert!(!wrong_mime.matches(&document, &[]));

        let wrong_sender = WorkflowConditions {
            correspondent_id: Some(Id::new_v4()),
            ..Default::default()
        };
        assert!(!wrong_sender.matches(&document, &[]));

        let wrong_type = WorkflowConditions {
            document_type_id: Some(Id::new_v4()),
            ..Default::default()
        };
        assert!(!wrong_type.matches(&document, &[]));
    }

    #[test]
    fn conditions_on_absent_fields_do_not_match() {
        let bare = Document::new(VaultId::new(), "t");
        let by_mime = WorkflowConditions {
            mime_type: Some("*".into()),
            ..Default::default()
        };
        let by_name = WorkflowConditions {
            filename_glob: Some("*".into()),
            ..Default::default()
        };
        assert!(!by_mime.matches(&bare, &[]));
        assert!(!by_name.matches(&bare, &[]));
    }

    fn webhook(url: &str) -> WorkflowAction {
        WorkflowAction::Webhook {
            url: url.to_string(),
            method: WebhookMethod::Post,
            headers: BTreeMap::new(),
            secret: None,
        }
    }

    #[test]
    fn trigger_selection_accepts_ordered_json_without_a_legacy_primary() {
        let input: WorkflowInput = serde_json::from_value(serde_json::json!({
            "name": "One or the other",
            "triggers": ["document.title_changed", "document.tags_changed"],
            "actions": [{"type": "webhook", "url": "https://receiver.test"}],
        }))
        .unwrap();
        assert_eq!(input.trigger, WorkflowTrigger::DocumentTitleChanged);
        input.validate().unwrap();
        let encoded = serde_json::to_value(input).unwrap();
        assert_eq!(
            encoded["triggers"],
            serde_json::json!(["document.title_changed", "document.tags_changed"])
        );
    }

    #[test]
    fn trigger_selection_rejects_empty_conflicting_or_duplicate_choices() {
        let mut body = serde_json::json!({
            "name": "Invalid trigger configuration",
            "trigger": "document.uploaded",
            "triggers": [],
            "actions": [{"type": "webhook", "url": "https://receiver.test"}],
        });
        assert!(serde_json::from_value::<WorkflowInput>(body.clone()).is_err());
        body["triggers"] = serde_json::json!(["document.deleted"]);
        assert!(serde_json::from_value::<WorkflowInput>(body.clone()).is_err());
        body["triggers"] = serde_json::json!(["document.uploaded", "document.uploaded"]);
        let duplicate: WorkflowInput = serde_json::from_value(body).unwrap();
        assert!(matches!(duplicate.validate(), Err(Error::Validation(_))));
    }

    #[test]
    fn trigger_selection_rejects_incompatible_secondary_events() {
        let input: WorkflowInput = serde_json::from_value(serde_json::json!({
            "name": "Unsupported tag workflow",
            "trigger": "document.uploaded",
            "triggers": ["document.uploaded", "document.deleted"],
            "actions": [{"type": "add_tags", "tag_ids": [Id::new_v4()]}],
        }))
        .unwrap();
        assert!(matches!(input.validate(), Err(Error::Validation(_))));
    }

    fn input(trigger: WorkflowTrigger, actions: Vec<WorkflowAction>) -> WorkflowInput {
        WorkflowInput {
            cadence: None,
            name: "n".into(),
            enabled: true,
            trigger,
            triggers: Vec::new(),
            conditions: Default::default(),
            actions,
        }
    }

    #[test]
    fn validation_allows_private_hosts_but_only_http_schemes() {
        for ok in [
            "http://192.168.1.10:8123/api/webhook/x",
            "http://localhost:5678/hook",
            "https://example.com/hook",
        ] {
            let wf = input(WorkflowTrigger::DocumentUploaded, vec![webhook(ok)]);
            assert!(wf.validate().is_ok(), "{ok}");
        }
        for bad in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "gopher://h",
            "not a url",
            "http://",
        ] {
            let wf = input(WorkflowTrigger::DocumentUploaded, vec![webhook(bad)]);
            assert!(wf.validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn validation_requires_a_name_and_actions() {
        let mut wf = input(WorkflowTrigger::DocumentUploaded, vec![]);
        assert!(wf.validate().is_err());
        wf.actions = vec![webhook("https://example.com")];
        wf.name = "  ".into();
        assert!(wf.validate().is_err());
    }

    #[test]
    fn deleted_trigger_only_allows_webhooks() {
        let local = WorkflowAction::AddTags {
            tag_ids: vec![Id::new_v4()],
        };
        let wf = input(WorkflowTrigger::DocumentDeleted, vec![local]);
        assert!(wf.validate().is_err());
    }

    #[test]
    fn actions_round_trip_through_json() {
        let json = r#"{"type":"webhook","url":"https://e.com","secret":"s"}"#;
        let action: WorkflowAction = serde_json::from_str(json).unwrap();
        assert_eq!(
            action,
            WorkflowAction::Webhook {
                url: "https://e.com".into(),
                method: WebhookMethod::Post,
                headers: BTreeMap::new(),
                secret: Some("s".into()),
            }
        );
        let back: WorkflowAction =
            serde_json::from_str(&serde_json::to_string(&action).unwrap()).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn redact_url_hides_credentials() {
        assert_eq!(
            redact_url("https://user:pw@example.com/hook?x=1"),
            "https://***@example.com/hook?x=1"
        );
        assert_eq!(redact_url("http://lan:8123/h"), "http://lan:8123/h");
        assert_eq!(redact_url("not a url"), "<invalid url>");
        let err = validate_webhook_url("ftp://user:pw@example.com/x").unwrap_err();
        assert!(!err.to_string().contains("pw"));
    }

    #[test]
    fn custom_metadata_triggers_use_their_occurrence_targets() {
        for event in [
            "metadata_field.created",
            "metadata_field.updated",
            "metadata_field.deleted",
            "metadata_field.restored",
            "metadata_option.created",
            "metadata_option.updated",
            "metadata_option.deleted",
            "metadata_option.restored",
            "document.metadata.changed",
        ] {
            let trigger = WorkflowTrigger::parse(event).expect(event);
            let target = if event.starts_with("document.") {
                WorkflowTarget::Document
            } else {
                WorkflowTarget::Event
            };
            assert_eq!(trigger.target(), target);
            let descriptor = super::super::WorkflowTriggerRegistry::built_in()
                .get(event)
                .expect(event);
            assert_eq!(descriptor.target, target);
            let workflow = input(
                trigger,
                vec![WorkflowAction::AddTags {
                    tag_ids: vec![Id::new_v4()],
                }],
            );
            assert_eq!(
                workflow.validate().is_ok(),
                target == WorkflowTarget::Document
            );
        }
    }

    #[test]
    fn departed_document_triggers_reject_document_mutations() {
        let action = WorkflowAction::AddTags { tag_ids: vec![] };
        for name in [
            "document.moved_out",
            "document.trashed",
            "document.purge_requested",
            "document.purged",
        ] {
            let trigger = WorkflowTrigger::parse(name).expect(name);
            assert_eq!(trigger.target(), WorkflowTarget::Event);
            assert!(!trigger.target().accepts(&action));
            assert!(!trigger.once_per_document());
        }
    }

    #[test]
    fn returning_document_triggers_support_document_actions() {
        let action = WorkflowAction::AddTags { tag_ids: vec![] };
        for name in ["document.moved_in", "document.restored"] {
            let trigger = WorkflowTrigger::parse(name).expect(name);
            assert_eq!(trigger.target(), WorkflowTarget::Document);
            assert!(trigger.target().accepts(&action));
            assert!(!trigger.once_per_document());
        }
    }

    #[test]
    fn purge_workflows_report_unavailable_document_actions() {
        let trigger = WorkflowTrigger::parse("document.purged").expect("purge trigger");
        let workflow = input(
            trigger,
            vec![WorkflowAction::AddTags {
                tag_ids: vec![Id::new_v4()],
            }],
        );
        assert_eq!(
            workflow.validate().unwrap_err().to_string(),
            "Validation error: Only webhook actions can run for this document lifecycle event"
        );
        assert!(input(trigger, vec![webhook("https://example.com/hook")])
            .validate()
            .is_ok());
    }

    #[test]
    fn trigger_wire_names_round_trip() {
        for t in WorkflowTrigger::ALL {
            assert_eq!(WorkflowTrigger::parse(t.as_str()), Some(t));
            assert_eq!(
                serde_json::to_string(&t).unwrap(),
                format!("\"{}\"", t.as_str())
            );
        }
    }
}
