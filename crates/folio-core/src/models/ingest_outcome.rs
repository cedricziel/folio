use serde::{Deserialize, Serialize};

use super::{Id, SourceKind, VaultId};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum IngestResult {
    Imported,
    Skipped,
    Duplicate,
    Failed,
}

impl IngestResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::Skipped => "skipped",
            Self::Duplicate => "duplicate",
            Self::Failed => "failed",
        }
    }

    pub fn event_type(self) -> &'static str {
        match self {
            Self::Imported => "ingest.imported",
            Self::Skipped => "ingest.skipped",
            Self::Duplicate => "ingest.duplicate",
            Self::Failed => "ingest.failed",
        }
    }

    pub fn reason_code(self) -> &'static str {
        match self {
            Self::Imported => "import_completed",
            Self::Skipped => "rule_skipped",
            Self::Duplicate => "content_duplicate",
            Self::Failed => "import_failed",
        }
    }

    pub fn retryable(self) -> bool {
        self == Self::Failed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceHealthChange {
    Failed,
    Recovered,
}

impl SourceHealthChange {
    /// Compares persisted failure state; an initial healthy poll is not a recovery.
    pub fn between(previously_failed: Option<bool>, failed: bool) -> Option<Self> {
        match (previously_failed, failed) {
            (None | Some(false), true) => Some(Self::Failed),
            (Some(true), false) => Some(Self::Recovered),
            _ => None,
        }
    }

    pub fn event_type(self) -> &'static str {
        match self {
            Self::Failed => "source.failed",
            Self::Recovered => "source.recovered",
        }
    }
}

#[derive(Debug, Clone)]
pub struct IngestAttempt {
    pub item_id: Id,
    pub attempt_id: Id,
    pub vault_id: VaultId,
    pub source_id: Id,
    pub source_kind: SourceKind,
    pub source_origin: super::SourceOrigin,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum IngestClaim {
    Claimed(IngestAttempt),
    Busy,
    Completed(IngestResult),
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IngestOutcomeRecord {
    pub id: i64,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub source_id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub item_id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub attempt_id: Id,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub event_id: Id,
    pub occurred_at: DateTime<Utc>,
    pub outcome: IngestResult,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub document_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IngestOutcomePage {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub vault_id: VaultId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub source_id: Id,
    pub outcomes: Vec<IngestOutcomeRecord>,
    pub next_before: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_codes_cannot_contain_provider_error_text() {
        assert!(serde_json::from_str::<IngestResult>(r#""password=secret""#).is_err());
        assert_eq!(
            serde_json::to_value(IngestResult::Failed).unwrap(),
            "failed"
        );
        assert_eq!(IngestResult::Failed.reason_code(), "import_failed");
        assert!(IngestResult::Failed.retryable());
        assert!(!IngestResult::Skipped.retryable());
    }

    #[test]
    fn repeated_source_failures_and_initial_health_emit_no_transition() {
        assert_eq!(SourceHealthChange::between(None, false), None);
        assert_eq!(SourceHealthChange::between(Some(true), true), None);
        assert_eq!(SourceHealthChange::between(Some(false), false), None);
        assert_eq!(
            SourceHealthChange::between(None, true),
            Some(SourceHealthChange::Failed)
        );
        assert_eq!(
            SourceHealthChange::between(Some(false), true),
            Some(SourceHealthChange::Failed)
        );
        assert_eq!(
            SourceHealthChange::between(Some(true), false),
            Some(SourceHealthChange::Recovered)
        );
    }
}
