use chrono::{DateTime, Duration, Utc};
use folio_core::models::{AttemptOutcome, ProcessingAttempt, StageOutcome, VaultId};
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::processing_events::{
    processing_envelope, ProcessingEventOutcome, ProcessingEventStage, ProcessingOccurrence,
    ProcessingReason,
};
use folio_core::{DomainEventEnvelope, Error, EventSnapshot, Result};
use uuid::Uuid;

use super::{FailOutcome, Job, JobType};

pub(super) mod postgres;
pub(super) mod sqlite;
#[cfg(test)]
mod tests;

#[derive(sqlx::FromRow)]
struct JobRow {
    job_type: String,
    payload: String,
    status: String,
    attempts: i32,
    max_attempts: i32,
    claim_id: Option<String>,
    live: bool,
}

impl JobRow {
    fn document_id(&self) -> Option<Uuid> {
        let job = serde_json::from_str::<JobType>(&self.payload).ok()?;
        if job.type_name() == self.job_type {
            job.document_id()
        } else {
            None
        }
    }
}

#[derive(sqlx::FromRow)]
struct StageRow {
    id: String,
    document_id: String,
    vault_id: String,
    stage: String,
    attempt: i64,
    correlation_id: String,
    causation_id: Option<String>,
}

impl StageRow {
    fn occurrence(&self, decision: &Decision) -> Result<ProcessingOccurrence> {
        let number = u32::try_from(self.attempt).map_err(repo_err)?;
        let mut attempt = ProcessingAttempt::restore(number, StageOutcome::Running)
            .ok_or_else(|| repo_err("Invalid persisted processing attempt"))?;
        let outcome = if decision.retry_at.is_some() {
            AttemptOutcome::Retry
        } else {
            AttemptOutcome::Failed
        };
        if !attempt.finish(number, outcome) {
            return Err(repo_err("Processing attempt cannot finish"));
        }
        Ok(ProcessingOccurrence {
            run_id: self.id.parse().map_err(repo_err)?,
            revision_id: None,
            stage: match self.stage.as_str() {
                "ocr" => ProcessingEventStage::Ocr,
                "embedding" => ProcessingEventStage::Embedding,
                "thumbnail" => ProcessingEventStage::Thumbnail,
                _ => return Err(repo_err("Invalid persisted processing stage")),
            },
            attempt: number,
            outcome: match decision.retry_at {
                Some(retry_at) => ProcessingEventOutcome::RetryScheduled {
                    reason: decision.reason,
                    retry_at,
                },
                None => ProcessingEventOutcome::Failed {
                    reason: decision.reason,
                },
            },
        })
    }

    fn vault(&self) -> Result<VaultId> {
        Ok(VaultId(self.vault_id.parse().map_err(repo_err)?))
    }

    fn document(&self) -> Result<Uuid> {
        self.document_id.parse().map_err(repo_err)
    }

    async fn envelope(
        &self,
        snapshot: &EventSnapshot,
        decision: &Decision,
    ) -> Result<DomainEventEnvelope> {
        let lineage = MutationLineage {
            correlation_id: self.correlation_id.parse().map_err(repo_err)?,
            causation_id: self
                .causation_id
                .as_ref()
                .map(|id| id.parse())
                .transpose()
                .map_err(repo_err)?,
        };
        with_actor(
            None,
            with_lineage(Some(lineage), async {
                processing_envelope(snapshot, &self.occurrence(decision)?)
            }),
        )
        .await
    }
}

enum Failure<'a> {
    Report {
        job: &'a Job,
        error: &'a str,
        retry: bool,
        reason: ProcessingReason,
    },
    Expired,
}

struct Decision {
    retry_at: Option<DateTime<Utc>>,
    reason: ProcessingReason,
    error: String,
}

impl Decision {
    fn outcome(&self) -> FailOutcome {
        if self.retry_at.is_some() {
            FailOutcome::Retrying
        } else {
            FailOutcome::GaveUp
        }
    }

    fn stage_state(&self) -> &'static str {
        if self.retry_at.is_some() {
            "waiting_to_retry"
        } else {
            "failed"
        }
    }

    fn reason_code(&self) -> Result<String> {
        serde_json::to_value(self.reason)
            .map_err(repo_err)?
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| repo_err("Invalid processing reason"))
    }
}

impl Failure<'_> {
    fn decision(&self, row: &JobRow, now: DateTime<Utc>) -> Result<Option<Decision>> {
        if row.status != "processing" {
            return Ok(None);
        }
        let (retry, reason, error) = match self {
            Self::Report {
                job,
                error,
                retry,
                reason,
            } => {
                if !row.live
                    || job.claim_id.is_none()
                    || row.attempts <= 0
                    || job.attempts != row.attempts
                    || job.claim_id.map(|id| id.to_string()) != row.claim_id
                {
                    return Ok(None);
                }
                (*retry, *reason, error.to_string())
            }
            Self::Expired => {
                if row.live {
                    return Ok(None);
                }
                (
                    true,
                    ProcessingReason::LeaseExpired,
                    "Processing lease expired".into(),
                )
            }
        };
        if matches!(
            reason,
            ProcessingReason::ProviderDisabled
                | ProcessingReason::NoText
                | ProcessingReason::UnsupportedFormat
        ) {
            return Err(Error::Validation(
                "Invalid processing failure reason".into(),
            ));
        }
        let retry_at = if retry && row.attempts < row.max_attempts {
            Some(match self {
                Self::Expired => now,
                Self::Report { .. } => 2_i64
                    .checked_pow(row.attempts.max(0) as u32)
                    .and_then(|factor| factor.checked_mul(10))
                    .and_then(Duration::try_seconds)
                    .and_then(|delay| now.checked_add_signed(delay))
                    .ok_or_else(|| repo_err("Processing retry schedule overflow"))?,
            })
        } else {
            None
        };
        Ok(Some(Decision {
            retry_at,
            reason,
            error,
        }))
    }

    fn expired(&self) -> bool {
        matches!(self, Self::Expired)
    }
}

fn repo_err(error: impl std::fmt::Display) -> Error {
    Error::Repository(error.to_string())
}
