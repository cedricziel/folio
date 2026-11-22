use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::StageOutcome;
use crate::processing_events::{ProcessingEventStage, ProcessingReason};

/// Current persisted stage state; absent legacy runs are never synthesized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ProcessingStageSummary {
    pub run_id: Uuid,
    /// Absent for runs that started before immutable revision identity was available.
    pub revision_id: Option<Uuid>,
    pub stage: ProcessingEventStage,
    /// Counts stage starts within this run; zero means the stage has not started.
    pub attempt: u32,
    pub state: StageOutcome,
    /// Allowlisted reason for a retry, skip, or terminal failure.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub reason_code: Option<ProcessingReason>,
    pub retry_at: Option<DateTime<Utc>>,
    /// Set only for completed OCR; `false` is a successful completion with no extracted text.
    pub has_text: Option<bool>,
}
