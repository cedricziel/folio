use chrono::Utc;
use folio_core::models::AttemptOutcome;
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::processing_events::{ProcessingEventOutcome, ProcessingReason};
use folio_core::{DocumentEvent, Error, EventOrigin, EventSnapshot, Result};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::database::postgres::{
    capture_document, insert_domain_event, insert_event_with_envelope,
};
use crate::queue::processing_scope::with_scope;
use crate::queue::{insert_job_postgres, Job, JobType, DEFAULT_MAX_ATTEMPTS};

use super::{
    claimed_ocr_document, embedding_skip_envelope, envelope, reason_code, repo_err, LoadedStage,
    OcrStageFence, StageRow,
};

async fn load(
    conn: &mut PgConnection,
    document_id: Uuid,
    job_id: Uuid,
    job_type: &'static str,
    claim_id: Uuid,
    queue_attempt: i32,
) -> Result<Option<LoadedStage>> {
    // Keep document, job, and run locking in this order across starts and completions.
    let document: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM documents WHERE id = $1 FOR UPDATE")
            .bind(document_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(repo_err)?;
    if document.is_none() {
        return Ok(None);
    }
    let job: Option<Uuid> = sqlx::query_scalar("SELECT id FROM jobs WHERE id = $1 FOR UPDATE")
        .bind(job_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(repo_err)?;
    if job.is_none() {
        return Ok(None);
    }
    // Revision-backed work needs the revision owner's persisted current-revision check.
    let row = sqlx::query_as::<_, StageRow>(
        "SELECT r.id::text AS run_id, r.vault_id::text, r.group_id::text, r.state, r.attempt,
                r.claim_id::text AS stage_claim_id, r.queue_attempt AS stage_queue_attempt,
                j.correlation_id::text, j.causation_id::text
         FROM documents d JOIN jobs j ON j.id = $1
         JOIN processing_stage_runs r ON r.document_id = d.id AND r.job_id = j.id
         JOIN vaults v ON v.id = d.vault_id AND v.deleted_at IS NULL
         WHERE d.id = $2 AND d.deleted_at IS NULL AND d.purge_requested_at IS NULL
           AND d.vault_id = j.origin_vault_id
           AND r.vault_id = j.origin_vault_id AND r.group_id = j.processing_group_id
           AND r.stage = 'ocr' AND r.is_current AND r.revision_id IS NULL
           AND j.status = 'processing' AND j.claim_id = $3 AND j.attempts = $4
           AND j.lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint
           AND j.correlation_id IS NOT NULL
           AND j.job_type = $5 AND j.payload->>'type' = CASE j.job_type
               WHEN 'process_document' THEN 'ProcessDocument' WHEN 'extract_text' THEN 'ExtractText' END
           AND j.payload->'data'->>'document_id' = d.id::text
         FOR UPDATE OF r FOR SHARE OF v",
    )
    .bind(job_id)
    .bind(document_id)
    .bind(claim_id)
    .bind(queue_attempt)
    .bind(job_type)
    .fetch_optional(conn)
    .await
    .map_err(repo_err)?;
    row.map(|row| row.into_stage(document_id, job_id, job_type, claim_id, queue_attempt))
        .transpose()
}

/// Start a seeded legacy OCR run, or return `None` for stale or terminal work.
/// Repeating a live claim returns the same fence without another event.
pub async fn start_ocr(pool: &PgPool, job: &Job) -> Result<Option<OcrStageFence>> {
    let Some((document_id, claim_id)) = claimed_ocr_document(job) else {
        return Ok(None);
    };
    let mut tx = pool.begin().await.map_err(repo_err)?;
    let Some(stage) = load(
        &mut tx,
        document_id,
        job.id,
        job.job_type.type_name(),
        claim_id,
        job.attempts,
    )
    .await?
    else {
        return Ok(None);
    };
    if stage.owns_running_attempt() {
        tx.commit().await.map_err(repo_err)?;
        return Ok(Some(stage.fence));
    }
    let Some(fence) = stage.next_fence() else {
        return Ok(None);
    };
    let now = Utc::now();
    let changed = sqlx::query(
        "UPDATE processing_stage_runs SET state = 'running', attempt = $1, queue_attempt = $2,
             claim_id = $3, reason_code = NULL, retry_at = NULL, updated_at = $4
         WHERE id = $5 AND EXISTS (SELECT 1 FROM jobs WHERE id = $6 AND claim_id = $3
             AND status = 'processing' AND attempts = $2
             AND lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint)",
    )
    .bind(i64::from(fence.attempt))
    .bind(job.attempts)
    .bind(claim_id)
    .bind(now)
    .bind(fence.run_id)
    .bind(job.id)
    .execute(&mut *tx)
    .await
    .map_err(repo_err)?;
    if changed.rows_affected() == 0 {
        return Ok(None);
    }
    sqlx::query("UPDATE documents SET ocr_status = 'running', updated_at = $1 WHERE id = $2")
        .bind(now)
        .bind(document_id)
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
    let snapshot = capture_document(&mut tx, document_id, Some(fence.vault_id))
        .await?
        .ok_or_else(|| repo_err("Fenced OCR document disappeared"))?;
    let event = envelope(
        &snapshot,
        &fence,
        stage.lineage,
        ProcessingEventOutcome::Started,
    )
    .await?;
    insert_domain_event(&mut tx, &event).await?;
    tx.commit().await.map_err(repo_err)?;
    Ok(Some(fence))
}

/// Commit OCR content, its compatibility/domain event, and planned embedding work atomically.
/// Duplicate, expired, moved, trashed, or superseded work returns `false`.
pub async fn complete_ocr(
    pool: &PgPool,
    fence: &OcrStageFence,
    content: &str,
    embed: bool,
) -> Result<bool> {
    finish_ocr(pool, fence, Some(content), embed).await
}

/// Skip disabled OCR while preserving content and atomically resolving its embedding plan.
pub async fn skip_ocr(pool: &PgPool, fence: &OcrStageFence, embed: bool) -> Result<bool> {
    finish_ocr(pool, fence, None, embed).await
}

async fn finish_ocr(
    pool: &PgPool,
    fence: &OcrStageFence,
    content: Option<&str>,
    embed: bool,
) -> Result<bool> {
    let mut tx = pool.begin().await.map_err(repo_err)?;
    let Some(stage) = load(
        &mut tx,
        fence.document_id,
        fence.job_id,
        fence.job_type,
        fence.claim_id,
        fence.queue_attempt,
    )
    .await?
    else {
        return Ok(false);
    };
    let completed = content.is_some();
    let outcome = if completed {
        AttemptOutcome::Completed
    } else {
        AttemptOutcome::Skipped
    };
    if !stage.accepts_finish(fence, outcome) {
        return Ok(false);
    }
    let now = Utc::now();
    let content = match content {
        Some(content) => Some(content.to_owned()),
        None => sqlx::query_scalar("SELECT content FROM documents WHERE id = $1")
            .bind(fence.document_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(repo_err)?,
    };
    let has_text = content.as_deref().is_some_and(|text| !text.is_empty());
    let embed = has_text && embed;
    let changed = sqlx::query(
        "UPDATE processing_stage_runs SET state = $7, claim_id = NULL,
             has_text = $1, reason_code = $8, updated_at = $2 WHERE id = $3
         AND EXISTS (SELECT 1 FROM jobs WHERE id = $4 AND claim_id = $5
             AND status = 'processing' AND attempts = $6
             AND lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint)",
    )
    .bind(completed.then_some(has_text))
    .bind(now)
    .bind(fence.run_id)
    .bind(fence.job_id)
    .bind(fence.claim_id)
    .bind(fence.queue_attempt)
    .bind(if completed { "completed" } else { "skipped" })
    .bind((!completed).then_some("provider_disabled"))
    .execute(&mut *tx)
    .await
    .map_err(repo_err)?;
    if changed.rows_affected() == 0 {
        return Ok(false);
    }
    // A newer processing group may own embedding; its status and job stay untouched.
    let owns_embedding: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM processing_stage_runs WHERE group_id = $1 AND document_id = $2
             AND stage = 'embedding' AND is_current AND revision_id IS NULL)",
    )
    .bind(fence.group_id)
    .bind(fence.document_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(repo_err)?;
    sqlx::query(
        "UPDATE documents SET content = $1, ocr_status = $5, embed_status = COALESCE($4, embed_status), updated_at = $2 WHERE id = $3",
    )
    .bind(&content)
    .bind(now)
    .bind(fence.document_id)
    .bind(owns_embedding.then_some(if embed { "pending" } else { "skipped" }))
    .bind(if completed { "done" } else { "skipped" })
    .execute(&mut *tx)
    .await
    .map_err(repo_err)?;
    let snapshot = capture_document(&mut tx, fence.document_id, Some(fence.vault_id))
        .await?
        .ok_or_else(|| repo_err("Fenced OCR document disappeared"))?;
    let event = envelope(
        &snapshot,
        fence,
        stage.lineage,
        if completed {
            ProcessingEventOutcome::Completed {
                has_text: Some(has_text),
            }
        } else {
            ProcessingEventOutcome::Skipped {
                reason: ProcessingReason::ProviderDisabled,
            }
        },
    )
    .await?;
    if completed {
        insert_event_with_envelope(
            &mut tx,
            &DocumentEvent::OcrComplete {
                document_id: fence.document_id,
            },
            EventOrigin::User,
            Some(&snapshot),
            &event,
        )
        .await?;
    } else {
        insert_domain_event(&mut tx, &event).await?;
    }
    let plan = if embed {
        None
    } else if has_text {
        Some(ProcessingReason::ProviderDisabled)
    } else {
        Some(ProcessingReason::NoText)
    };
    if owns_embedding {
        complete_embedding_plan(&mut tx, fence, &snapshot, stage.lineage, plan).await?;
    }
    tx.commit().await.map_err(repo_err)?;
    Ok(true)
}

async fn complete_embedding_plan(
    conn: &mut PgConnection,
    fence: &OcrStageFence,
    snapshot: &EventSnapshot,
    lineage: MutationLineage,
    skip: Option<ProcessingReason>,
) -> Result<()> {
    let Some(reason) = skip else {
        with_actor(
            None,
            with_lineage(
                Some(lineage),
                with_scope(Some(fence.scope()), async {
                    insert_job_postgres(
                        conn,
                        JobType::generate_embedding(fence.document_id),
                        Utc::now(),
                        DEFAULT_MAX_ATTEMPTS,
                        crate::queue::JobTraceContext::current().as_ref(),
                    )
                    .await
                }),
            ),
        )
        .await?;
        return Ok(());
    };
    let run_id: Option<Uuid> = sqlx::query_scalar(
        "UPDATE processing_stage_runs SET state = 'skipped', attempt = 1, queue_attempt = $1,
             job_id = COALESCE(job_id, $2), claim_id = NULL, reason_code = $8,
             retry_at = NULL, updated_at = $3
         WHERE group_id = $4 AND document_id = $5 AND vault_id = $6 AND stage = 'embedding'
           AND state = 'pending' AND attempt = 0 AND is_current AND revision_id IS NULL
           AND EXISTS (SELECT 1 FROM jobs WHERE id = $2 AND claim_id = $7 AND attempts = $1
               AND status = 'processing' AND lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint)
         RETURNING id",
    )
    .bind(fence.queue_attempt)
    .bind(fence.job_id)
    .bind(Utc::now())
    .bind(fence.group_id)
    .bind(fence.document_id)
    .bind(fence.vault_id.0)
    .bind(fence.claim_id)
    .bind(reason_code(reason)?)
    .fetch_optional(&mut *conn)
    .await
    .map_err(repo_err)?;
    let run_id = run_id
        .ok_or_else(|| Error::Conflict("Pending embedding run or OCR lease changed".into()))?;
    let event = embedding_skip_envelope(snapshot, run_id, lineage, reason).await?;
    insert_event_with_envelope(
        conn,
        &DocumentEvent::Processed {
            document_id: fence.document_id,
        },
        EventOrigin::User,
        Some(snapshot),
        &event,
    )
    .await?;
    Ok(())
}
