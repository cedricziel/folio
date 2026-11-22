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

/// A started processing attempt, usable only while its persisted queue claim remains live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageFence {
    stage: ProcessingEventStage,
    run_id: Uuid,
    document_id: Uuid,
    revision_id: Option<Uuid>,
    vault_id: VaultId,
    pub(super) group_id: Uuid,
    job_id: Uuid,
    job_type: &'static str,
    claim_id: Uuid,
    queue_attempt: i32,
    attempt: u32,
}

/// Compatibility name for OCR callers.
pub type OcrStageFence = StageFence;

impl StageFence {
    /// Immutable object namespace for results produced by this claim.
    pub fn result_prefix(&self) -> String {
        format!(
            "documents/{}/processing/{}/{}",
            self.document_id, self.run_id, self.claim_id
        )
    }

    /// The logical document fenced by this attempt.
    pub fn document_id(&self) -> Uuid {
        self.document_id
    }

    /// The enqueue-time vault, retained even if the document is later moved.
    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    fn embedding_job(&self) -> JobType {
        match self.revision_id {
            Some(revision_id) => JobType::GenerateRevisionEmbedding {
                document_id: self.document_id,
                revision_id,
                origin: folio_core::EventOrigin::User,
            },
            None => JobType::generate_embedding(self.document_id),
        }
    }

    fn scope(&self) -> ProcessingScope {
        ProcessingScope {
            group_id: self.group_id,
            document_id: self.document_id,
            vault_id: self.vault_id,
            revision_id: self.revision_id,
            parent_job_id: self.job_id,
            claim_id: self.claim_id,
            queue_attempt: self.queue_attempt,
        }
    }
}

#[derive(sqlx::FromRow)]
struct StageRow {
    run_id: String,
    stage: String,
    payload: String,
    vault_id: String,
    revision_id: Option<String>,
    group_id: String,
    state: String,
    attempt: i64,
    stage_claim_id: Option<String>,
    stage_queue_attempt: i32,
    correlation_id: String,
    causation_id: Option<String>,
}

struct LoadedStage {
    origin: folio_core::EventOrigin,
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
            .ok_or_else(|| repo_err("Invalid persisted processing attempt"))?;
        Ok(LoadedStage {
            origin: match serde_json::from_str::<JobType>(&self.payload).map_err(repo_err)? {
                JobType::GenerateEmbedding { origin, .. }
                | JobType::GenerateRevisionEmbedding { origin, .. } => origin,
                _ => folio_core::EventOrigin::User,
            },
            fence: StageFence {
                stage: match self.stage.as_str() {
                    "ocr" => ProcessingEventStage::Ocr,
                    "embedding" => ProcessingEventStage::Embedding,
                    "thumbnail" => ProcessingEventStage::Thumbnail,
                    _ => return Err(repo_err("Invalid processing stage")),
                },
                run_id: self.run_id.parse().map_err(repo_err)?,
                document_id,
                revision_id: self
                    .revision_id
                    .map(|id| id.parse())
                    .transpose()
                    .map_err(repo_err)?,
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
    claimed_stage_document(job, ProcessingEventStage::Ocr)
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
            revision_id: fence.revision_id,
            stage: fence.stage,
            attempt: fence.attempt,
            outcome,
        },
    )
    .await
}

async fn embedding_skip_envelope(
    snapshot: &EventSnapshot,
    run_id: Uuid,
    revision_id: Option<Uuid>,
    lineage: MutationLineage,
    reason: ProcessingReason,
) -> Result<DomainEventEnvelope> {
    occurrence_envelope(
        snapshot,
        lineage,
        &ProcessingOccurrence {
            run_id,
            revision_id,
            stage: ProcessingEventStage::Embedding,
            attempt: 1,
            outcome: ProcessingEventOutcome::Skipped { reason },
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

fn reason_code(reason: ProcessingReason) -> Result<String> {
    serde_json::to_value(reason)
        .map_err(repo_err)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| repo_err("Invalid processing reason"))
}

fn repo_err(error: impl std::fmt::Display) -> Error {
    Error::Repository(error.to_string())
}

pub(super) fn supports_stage(job: &JobType, stage: ProcessingEventStage) -> bool {
    use JobType::*;
    use ProcessingEventStage::*;
    matches!(
        (job, stage),
        (
            ProcessDocument { .. } | ProcessDocumentRevision { .. },
            Ocr | Thumbnail
        ) | (ExtractText { .. } | ExtractRevisionText { .. }, Ocr)
            | (
                GenerateEmbedding { .. } | GenerateRevisionEmbedding { .. },
                Embedding
            )
            | (
                GenerateThumbnail { .. } | GenerateRevisionThumbnail { .. },
                Thumbnail
            )
    )
}

fn claimed_stage_document(job: &Job, stage: ProcessingEventStage) -> Option<(Uuid, Uuid)> {
    if job.status != JobStatus::Processing
        || job.attempts <= 0
        || !supports_stage(&job.job_type, stage)
    {
        return None;
    }
    Some((job.job_type.document_id()?, job.claim_id?))
}

fn stage_name(stage: ProcessingEventStage) -> &'static str {
    match stage {
        ProcessingEventStage::Ocr => "ocr",
        ProcessingEventStage::Embedding => "embedding",
        ProcessingEventStage::Thumbnail => "thumbnail",
    }
}

enum DerivedResult<'a> {
    Embedding {
        expected_content: &'a str,
        model: &'a str,
        chunks: &'a [folio_core::ports::EmbeddedChunk],
    },
    Thumbnail {
        page_count: Option<u32>,
    },
    Skipped(ProcessingReason),
}

fn validate_chunks(model: &str, chunks: &[folio_core::ports::EmbeddedChunk]) -> Result<()> {
    let dimensions = chunks
        .first()
        .map(|chunk| chunk.embedding.len())
        .unwrap_or(0);
    if model.trim().is_empty()
        || dimensions == 0
        || chunks.iter().enumerate().any(|(i, chunk)| {
            chunk.index as usize != i
                || chunk.embedding.len() != dimensions
                || chunk.embedding.iter().any(|value| !value.is_finite())
        })
    {
        return Err(repo_err("Invalid embedding result"));
    }
    Ok(())
}
