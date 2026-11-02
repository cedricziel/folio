use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::UserId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ReviewOrigin {
    Human,
    System,
    LegacyClassification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewDecision {
    pub reviewed_at: DateTime<Utc>,
    pub reviewed_by: Option<UserId>,
    pub origin: ReviewOrigin,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentReview {
    decision: Option<ReviewDecision>,
}

impl DocumentReview {
    pub fn decision(&self) -> Option<&ReviewDecision> {
        self.decision.as_ref()
    }

    /// Records migration provenance, not a historical human review time.
    pub fn legacy_classification(migrated_at: DateTime<Utc>) -> Self {
        Self {
            decision: Some(ReviewDecision {
                reviewed_at: migrated_at,
                reviewed_by: None,
                origin: ReviewOrigin::LegacyClassification,
            }),
        }
    }

    /// Returns whether state changed. Actor and time must come from server context.
    pub fn set_reviewed(
        &mut self,
        reviewed: bool,
        actor: Option<UserId>,
        now: DateTime<Utc>,
    ) -> bool {
        if reviewed == self.decision.is_some() {
            return false;
        }
        self.decision = reviewed.then(|| ReviewDecision {
            reviewed_at: now,
            origin: if actor.is_some() {
                ReviewOrigin::Human
            } else {
                ReviewOrigin::System
            },
            reviewed_by: actor,
        });
        true
    }
}
