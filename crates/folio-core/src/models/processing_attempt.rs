use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum StageOutcome {
    #[default]
    Pending,
    Running,
    WaitingToRetry,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    Retry,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessingAttempt {
    number: u32,
    outcome: StageOutcome,
}

impl ProcessingAttempt {
    /// Restores persisted state; only pending runs can have zero attempts.
    pub fn restore(number: u32, outcome: StageOutcome) -> Option<Self> {
        if (number == 0) != (outcome == StageOutcome::Pending) {
            return None;
        }
        Some(Self { number, outcome })
    }

    /// Starts the next attempt only while the run is pending or waiting to retry.
    /// Database callers must also fence the lease, revision, and vault in their transaction.
    pub fn start(&mut self, number: u32) -> bool {
        if !matches!(
            self.outcome,
            StageOutcome::Pending | StageOutcome::WaitingToRetry
        ) || self.number.checked_add(1) != Some(number)
        {
            return false;
        }
        self.number = number;
        self.outcome = StageOutcome::Running;
        true
    }

    /// Returns false for duplicate, stale, or unstarted outcomes without changing state.
    pub fn finish(&mut self, number: u32, outcome: AttemptOutcome) -> bool {
        if self.outcome != StageOutcome::Running || self.number != number {
            return false;
        }
        self.outcome = match outcome {
            AttemptOutcome::Retry => StageOutcome::WaitingToRetry,
            AttemptOutcome::Completed => StageOutcome::Completed,
            AttemptOutcome::Failed => StageOutcome::Failed,
            AttemptOutcome::Skipped => StageOutcome::Skipped,
        };
        true
    }

    pub fn outcome(&self) -> StageOutcome {
        self.outcome
    }

    pub fn number(&self) -> u32 {
        self.number
    }
}
