pub mod postgres;
pub mod sqlite;

use folio_core::models::{AttemptOutcome, ProcessingAttempt, StageOutcome, VaultId};
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::processing_events::{
    processing_envelope, ProcessingEventOutcome, ProcessingEventStage, ProcessingOccurrence,
    ProcessingReason,
};
use folio_core::{DomainEventEnvelope, Error, EventSnapshot, Result};
use uuid::Uuid;

use super::processing_scope::ProcessingScope;
use super::{Job, JobStatus, JobType};

/// A started OCR attempt, usable only while its persisted queue claim remains live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrStageFence {
    run_id: Uuid,
    document_id: Uuid,
    vault_id: VaultId,
    group_id: Uuid,
    job_id: Uuid,
    job_type: &'static str,
    claim_id: Uuid,
    queue_attempt: i32,
    attempt: u32,
}

impl OcrStageFence {
    /// The logical document fenced by this attempt.
    pub fn document_id(&self) -> Uuid {
        self.document_id
    }

    /// The enqueue-time vault, retained even if the document is later moved.
    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    fn scope(&self) -> ProcessingScope {
        ProcessingScope {
            group_id: self.group_id,
            document_id: self.document_id,
            vault_id: self.vault_id,
            parent_job_id: self.job_id,
            claim_id: self.claim_id,
            queue_attempt: self.queue_attempt,
        }
    }
}

#[derive(sqlx::FromRow)]
struct StageRow {
    run_id: String,
    vault_id: String,
    group_id: String,
    state: String,
    attempt: i64,
    stage_claim_id: Option<String>,
    stage_queue_attempt: i32,
    correlation_id: String,
    causation_id: Option<String>,
}

struct LoadedStage {
    fence: OcrStageFence,
    attempt_state: ProcessingAttempt,
    stage_claim_id: Option<Uuid>,
    stage_queue_attempt: i32,
    lineage: MutationLineage,
}

impl StageRow {
    fn into_stage(
        self,
        document_id: Uuid,
        job_id: Uuid,
        job_type: &'static str,
        claim_id: Uuid,
        queue_attempt: i32,
    ) -> Result<LoadedStage> {
        let number = self.attempt.try_into().map_err(repo_err)?;
        let outcome =
            serde_json::from_value(serde_json::Value::String(self.state)).map_err(repo_err)?;
        let attempt_state = ProcessingAttempt::restore(number, outcome)
            .ok_or_else(|| repo_err("Invalid persisted OCR attempt"))?;
        Ok(LoadedStage {
            fence: OcrStageFence {
                run_id: self.run_id.parse().map_err(repo_err)?,
                document_id,
                vault_id: VaultId(self.vault_id.parse().map_err(repo_err)?),
                group_id: self.group_id.parse().map_err(repo_err)?,
                job_id,
                job_type,
                claim_id,
                queue_attempt,
                attempt: number,
            },
            attempt_state,
            stage_claim_id: self
                .stage_claim_id
                .map(|id| id.parse())
                .transpose()
                .map_err(repo_err)?,
            stage_queue_attempt: self.stage_queue_attempt,
            lineage: MutationLineage {
                correlation_id: self.correlation_id.parse().map_err(repo_err)?,
                causation_id: self
                    .causation_id
                    .map(|id| id.parse())
                    .transpose()
                    .map_err(repo_err)?,
            },
        })
    }
}

impl LoadedStage {
    fn owns_running_attempt(&self) -> bool {
        self.attempt_state.outcome() == StageOutcome::Running
            && self.stage_claim_id == Some(self.fence.claim_id)
            && self.stage_queue_attempt == self.fence.queue_attempt
    }

    fn next_fence(&self) -> Option<OcrStageFence> {
        let number = self.attempt_state.number().checked_add(1)?;
        let mut attempt = self.attempt_state.clone();
        if !attempt.start(number) {
            return None;
        }
        let mut fence = self.fence.clone();
        fence.attempt = attempt.number();
        Some(fence)
    }

    fn accepts_finish(&self, fence: &OcrStageFence, outcome: AttemptOutcome) -> bool {
        let mut attempt = self.attempt_state.clone();
        self.owns_running_attempt()
            && self.fence == *fence
            && attempt.finish(fence.attempt, outcome)
    }
}

fn claimed_ocr_document(job: &Job) -> Option<(Uuid, Uuid)> {
    if job.status != JobStatus::Processing || job.attempts <= 0 {
        return None;
    }
    let document_id = match job.job_type {
        JobType::ExtractText { document_id } | JobType::ProcessDocument { document_id } => {
            document_id
        }
        _ => return None,
    };
    Some((document_id, job.claim_id?))
}

async fn envelope(
    snapshot: &EventSnapshot,
    fence: &OcrStageFence,
    lineage: MutationLineage,
    outcome: ProcessingEventOutcome,
) -> Result<DomainEventEnvelope> {
    occurrence_envelope(
        snapshot,
        lineage,
        &ProcessingOccurrence {
            run_id: fence.run_id,
            revision_id: None,
            stage: ProcessingEventStage::Ocr,
            attempt: fence.attempt,
            outcome,
        },
    )
    .await
}

async fn embedding_skip_envelope(
    snapshot: &EventSnapshot,
    run_id: Uuid,
    lineage: MutationLineage,
) -> Result<DomainEventEnvelope> {
    occurrence_envelope(
        snapshot,
        lineage,
        &ProcessingOccurrence {
            run_id,
            revision_id: None,
            stage: ProcessingEventStage::Embedding,
            attempt: 1,
            outcome: ProcessingEventOutcome::Skipped {
                reason: ProcessingReason::NoText,
            },
        },
    )
    .await
}

async fn occurrence_envelope(
    snapshot: &EventSnapshot,
    lineage: MutationLineage,
    occurrence: &ProcessingOccurrence,
) -> Result<DomainEventEnvelope> {
    with_actor(
        None,
        with_lineage(Some(lineage), async {
            processing_envelope(snapshot, occurrence)
        }),
    )
    .await
}

fn repo_err(error: impl std::fmt::Display) -> Error {
    Error::Repository(error.to_string())
}
