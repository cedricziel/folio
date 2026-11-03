use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::events::document_event_envelope;
use crate::{DomainEventEnvelope, DomainEventSubject, Error, EventAudience, EventSnapshot, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingEventStage {
    Ocr,
    Embedding,
    Thumbnail,
}

impl ProcessingEventStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ocr => "ocr",
            Self::Embedding => "embedding",
            Self::Thumbnail => "thumbnail",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingReason {
    ProviderDisabled,
    NoText,
    UnsupportedFormat,
    ProviderFailed,
    InvalidResponse,
    StorageFailed,
    LeaseExpired,
}

#[derive(Debug, Clone, Copy)]
pub enum ProcessingEventOutcome {
    Started,
    Completed {
        has_text: Option<bool>,
    },
    Failed {
        reason: ProcessingReason,
    },
    Skipped {
        reason: ProcessingReason,
    },
    RetryScheduled {
        reason: ProcessingReason,
        retry_at: DateTime<Utc>,
    },
}

impl ProcessingEventOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed { .. } => "completed",
            Self::Failed { .. } => "failed",
            Self::Skipped { .. } => "skipped",
            Self::RetryScheduled { .. } => "retry_scheduled",
        }
    }
}

pub struct ProcessingOccurrence {
    pub run_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub stage: ProcessingEventStage,
    pub attempt: u32,
    pub outcome: ProcessingEventOutcome,
}

/// Call after the transaction has fenced the run and suppressed duplicate transitions.
/// The persisted envelope, not a newly built one, is reused on delivery retries.
pub fn processing_envelope(
    snapshot: &EventSnapshot,
    occurrence: &ProcessingOccurrence,
) -> Result<DomainEventEnvelope> {
    let invalid = || Error::Validation("Invalid processing occurrence".into());
    if occurrence.run_id.is_nil()
        || occurrence.revision_id.is_some_and(|id| id.is_nil())
        || occurrence.attempt == 0
    {
        return Err(invalid());
    }
    let mut data = document_event_envelope(
        &crate::DocumentEvent::Updated {
            document_id: snapshot.document_id,
        },
        Some(snapshot),
    )?
    .ok_or_else(invalid)?
    .data()
    .clone();
    data["payload_version"] = 1.into();
    data["run_id"] = serde_json::json!(occurrence.run_id);
    data["revision_id"] = serde_json::json!(occurrence.revision_id);
    data["stage"] = serde_json::json!(occurrence.stage);
    data["attempt"] = occurrence.attempt.into();
    data["outcome"] = occurrence.outcome.as_str().into();
    match occurrence.outcome {
        ProcessingEventOutcome::Started => {}
        ProcessingEventOutcome::Completed { has_text } => {
            if (occurrence.stage == ProcessingEventStage::Ocr) != has_text.is_some() {
                return Err(invalid());
            }
            if let Some(has_text) = has_text {
                data["has_text"] = has_text.into();
            }
        }
        ProcessingEventOutcome::Skipped { reason } => {
            let valid = match reason {
                ProcessingReason::ProviderDisabled => true,
                ProcessingReason::NoText => occurrence.stage == ProcessingEventStage::Embedding,
                ProcessingReason::UnsupportedFormat => {
                    occurrence.stage == ProcessingEventStage::Thumbnail
                }
                _ => false,
            };
            if !valid {
                return Err(invalid());
            }
            data["reason_code"] = serde_json::json!(reason);
        }
        ProcessingEventOutcome::Failed { reason }
        | ProcessingEventOutcome::RetryScheduled { reason, .. } => {
            if matches!(
                reason,
                ProcessingReason::ProviderDisabled
                    | ProcessingReason::NoText
                    | ProcessingReason::UnsupportedFormat
            ) {
                return Err(invalid());
            }
            data["reason_code"] = serde_json::json!(reason);
            if let ProcessingEventOutcome::RetryScheduled { retry_at, .. } = occurrence.outcome {
                data["retry_at"] = serde_json::json!(retry_at);
            }
        }
    }
    DomainEventEnvelope::new(
        format!(
            "document.processing.{}.{}",
            occurrence.stage.as_str(),
            occurrence.outcome.as_str()
        ),
        snapshot.vault_id,
        DomainEventSubject {
            subject_type: "document".into(),
            id: snapshot.document_id,
        },
        Some(snapshot.document_id),
        EventAudience::Vault,
        data,
    )
}
