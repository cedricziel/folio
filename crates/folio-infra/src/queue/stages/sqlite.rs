use chrono::{SecondsFormat, Utc};
use folio_core::models::AttemptOutcome;
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::processing_events::{ProcessingEventOutcome, ProcessingReason};
use folio_core::{DocumentEvent, Error, EventOrigin, EventSnapshot, Result};
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

use crate::database::sqlite::{
    capture_document, insert_domain_event_with_origin, insert_event_with_envelope,
};
use crate::queue::processing_scope::with_scope;
use crate::queue::{insert_job_sqlite, Job, DEFAULT_MAX_ATTEMPTS};

use super::{
    claimed_ocr_document, embedding_skip_envelope, envelope, reason_code, repo_err, LoadedStage,
    OcrStageFence, StageRow,
};

async fn load(
    conn: &mut SqliteConnection,
    document_id: Uuid,
    job_id: Uuid,
    job_type: &'static str,
    claim_id: Uuid,
    queue_attempt: i32,
    stage: super::ProcessingEventStage,
) -> Result<Option<LoadedStage>> {
    let row = sqlx::query_as::<_, StageRow>(
        "SELECT r.id AS run_id, r.vault_id, r.group_id, r.stage, r.state, r.attempt, j.payload,
                r.revision_id, r.claim_id AS stage_claim_id, r.queue_attempt AS stage_queue_attempt,
                j.correlation_id, j.causation_id
         FROM documents d JOIN jobs j ON j.id = ?
         JOIN processing_stage_runs r ON r.document_id = d.id AND r.job_id = j.id
         JOIN vaults v ON v.id = d.vault_id AND v.deleted_at IS NULL
         WHERE d.id = ? AND d.deleted_at IS NULL AND d.purge_requested_at IS NULL
           AND d.vault_id = j.origin_vault_id
           AND r.vault_id = j.origin_vault_id AND r.group_id = j.processing_group_id
           AND r.stage = ?6 AND r.is_current = 1
           AND r.revision_id IS json_extract(j.payload, '$.data.revision_id')
           AND ((r.revision_id IS NULL AND d.current_revision_id IS NULL AND NOT EXISTS (
               SELECT 1 FROM document_revisions history WHERE history.document_id = d.id
           )) OR EXISTS (
               SELECT 1 FROM document_revisions revision
               WHERE revision.id = d.current_revision_id AND revision.document_id = d.id
                 AND revision.vault_id = d.vault_id
                 AND (revision.id = r.revision_id OR (r.revision_id IS NULL AND revision.number = 1))
           ))
           AND j.status = 'processing' AND j.claim_id = ?3 AND j.attempts = ?4
           AND j.lease_expires_at > CAST(strftime('%s', 'now') AS INTEGER)
           AND j.correlation_id IS NOT NULL
           AND j.job_type = ?5 AND json_extract(j.payload, '$.type') = CASE j.job_type
               WHEN 'process_document' THEN 'ProcessDocument' WHEN 'extract_text' THEN 'ExtractText'
               WHEN 'generate_embedding' THEN 'GenerateEmbedding' WHEN 'generate_thumbnail' THEN 'GenerateThumbnail'
               WHEN 'process_document_revision' THEN 'ProcessDocumentRevision'
               WHEN 'extract_revision_text' THEN 'ExtractRevisionText'
               WHEN 'generate_revision_embedding' THEN 'GenerateRevisionEmbedding'
               WHEN 'generate_revision_thumbnail' THEN 'GenerateRevisionThumbnail' END
           AND json_extract(j.payload, '$.data.document_id') = d.id",
    )
    .bind(job_id.to_string())
    .bind(document_id.to_string())
    .bind(claim_id.to_string())
    .bind(queue_attempt)
    .bind(job_type)
    .bind(super::stage_name(stage))
    .fetch_optional(conn)
    .await
    .map_err(repo_err)?;
    row.map(|row| row.into_stage(document_id, job_id, job_type, claim_id, queue_attempt))
        .transpose()
}

/// Start a seeded OCR run, or return `None` for stale or terminal work.
/// Repeating a live claim returns the same fence without another event.
pub async fn start_ocr(pool: &SqlitePool, job: &Job) -> Result<Option<OcrStageFence>> {
    let Some((document_id, claim_id)) = claimed_ocr_document(job) else {
        return Ok(None);
    };
    start_stage(
        pool,
        job,
        super::ProcessingEventStage::Ocr,
        document_id,
        claim_id,
    )
    .await
}

/// Start a seeded embedding or thumbnail run; stale and terminal claims return `None`.
pub async fn start_derived(
    pool: &SqlitePool,
    job: &Job,
    stage: super::ProcessingEventStage,
) -> Result<Option<super::StageFence>> {
    let Some((document_id, claim_id)) = super::claimed_stage_document(job, stage) else {
        return Ok(None);
    };
    start_stage(pool, job, stage, document_id, claim_id).await
}

async fn start_stage(
    pool: &SqlitePool,
    job: &Job,
    stage_kind: super::ProcessingEventStage,
    document_id: Uuid,
    claim_id: Uuid,
) -> Result<Option<super::StageFence>> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(repo_err)?;
    let Some(stage) = load(
        &mut tx,
        document_id,
        job.id,
        job.job_type.type_name(),
        claim_id,
        job.attempts,
        stage_kind,
    )
    .await?
    else {
        return Ok(None);
    };
    if stage.fence.revision_id != job.job_type.revision_id() {
        return Ok(None);
    }
    if stage.owns_running_attempt() {
        if !live_claim(&mut tx, &stage.fence).await? {
            return Ok(None);
        }
        tx.commit().await.map_err(repo_err)?;
        return Ok(Some(stage.fence));
    }
    let Some(fence) = stage.next_fence() else {
        return Ok(None);
    };
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true);
    let changed = sqlx::query(
        "UPDATE processing_stage_runs SET state = 'running', attempt = ?, queue_attempt = ?,
             claim_id = ?, reason_code = NULL, retry_at = NULL, updated_at = ?
         WHERE id = ? AND EXISTS (SELECT 1 FROM jobs WHERE id = ? AND claim_id = ?
             AND status = 'processing' AND attempts = ?
             AND lease_expires_at > CAST(strftime('%s', 'now') AS INTEGER))",
    )
    .bind(i64::from(fence.attempt))
    .bind(job.attempts)
    .bind(claim_id.to_string())
    .bind(&now)
    .bind(fence.run_id.to_string())
    .bind(job.id.to_string())
    .bind(claim_id.to_string())
    .bind(job.attempts)
    .execute(&mut *tx)
    .await
    .map_err(repo_err)?;
    if changed.rows_affected() == 0 {
        return Ok(None);
    }
    sqlx::query("UPDATE documents SET ocr_status = CASE WHEN ? = 'ocr' THEN 'running' ELSE ocr_status END, embed_status = CASE WHEN ? = 'embedding' THEN 'running' ELSE embed_status END, updated_at = ? WHERE id = ?")
        .bind(super::stage_name(stage_kind))
        .bind(super::stage_name(stage_kind))
        .bind(&now)
        .bind(document_id.to_string())
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
    insert_domain_event_with_origin(&mut tx, &event, stage.origin).await?;
    if !live_claim(&mut tx, &fence).await? {
        return Ok(None);
    }
    tx.commit().await.map_err(repo_err)?;
    Ok(Some(fence))
}

/// Commit OCR content, its compatibility/domain event, and planned embedding work atomically.
/// Duplicate, expired, moved, trashed, or superseded work returns `false`.
pub async fn complete_ocr(
    pool: &SqlitePool,
    fence: &OcrStageFence,
    content: &str,
    embed: bool,
) -> Result<bool> {
    finish_ocr(pool, fence, Some(content), embed).await
}

/// Skip disabled OCR while preserving content and atomically resolving its embedding plan.
pub async fn skip_ocr(pool: &SqlitePool, fence: &OcrStageFence, embed: bool) -> Result<bool> {
    finish_ocr(pool, fence, None, embed).await
}

async fn finish_ocr(
    pool: &SqlitePool,
    fence: &OcrStageFence,
    content: Option<&str>,
    embed: bool,
) -> Result<bool> {
    if fence.stage != super::ProcessingEventStage::Ocr {
        return Ok(false);
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(repo_err)?;
    let Some(stage) = load(
        &mut tx,
        fence.document_id,
        fence.job_id,
        fence.job_type,
        fence.claim_id,
        fence.queue_attempt,
        fence.stage,
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
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true);
    let prior_content: Option<String> =
        sqlx::query_scalar("SELECT content FROM documents WHERE id = ?")
            .bind(fence.document_id.to_string())
            .fetch_one(&mut *tx)
            .await
            .map_err(repo_err)?;
    let content = content.map(str::to_owned).or_else(|| prior_content.clone());
    if content != prior_content {
        sqlx::query("DELETE FROM document_chunks WHERE document_id = ?")
            .bind(fence.document_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
    }
    let has_text = content.as_deref().is_some_and(|text| !text.is_empty());
    let embed = has_text && embed;
    let changed = sqlx::query(
        "UPDATE processing_stage_runs SET state = ?, claim_id = NULL,
             has_text = ?, reason_code = ?, updated_at = ? WHERE id = ?
         AND EXISTS (SELECT 1 FROM jobs WHERE id = ? AND claim_id = ?
             AND status = 'processing' AND attempts = ?
             AND lease_expires_at > CAST(strftime('%s', 'now') AS INTEGER))",
    )
    .bind(if completed { "completed" } else { "skipped" })
    .bind(completed.then_some(has_text))
    .bind((!completed).then_some("provider_disabled"))
    .bind(&now)
    .bind(fence.run_id.to_string())
    .bind(fence.job_id.to_string())
    .bind(fence.claim_id.to_string())
    .bind(fence.queue_attempt)
    .execute(&mut *tx)
    .await
    .map_err(repo_err)?;
    if changed.rows_affected() == 0 {
        return Ok(false);
    }
    let mut owns_embedding: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM processing_stage_runs WHERE group_id = ? AND document_id = ?
             AND vault_id = ? AND revision_id IS ?
             AND stage = 'embedding' AND is_current = 1)",
    )
    .bind(fence.group_id.to_string())
    .bind(fence.document_id.to_string())
    .bind(fence.vault_id.to_string())
    .bind(fence.revision_id.map(|id| id.to_string()))
    .fetch_one(&mut *tx)
    .await
    .map_err(repo_err)?;
    if !owns_embedding && content != prior_content {
        let unassigned: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM processing_stage_runs WHERE group_id = ? AND document_id = ? AND vault_id = ? AND stage = 'embedding' AND state = 'pending' AND attempt = 0 AND job_id IS NULL AND revision_id IS ?)")
            .bind(fence.group_id.to_string()).bind(fence.document_id.to_string()).bind(fence.vault_id.to_string()).bind(fence.revision_id.map(|id| id.to_string()))
            .fetch_one(&mut *tx).await.map_err(repo_err)?;
        if !unassigned {
            return Err(Error::Conflict(
                "Embedding plan changed before OCR completion".into(),
            ));
        }
        sqlx::query("UPDATE processing_stage_runs SET is_current = FALSE, updated_at = ? WHERE document_id = ? AND stage = 'embedding' AND is_current = TRUE")
            .bind(&now).bind(fence.document_id.to_string()).execute(&mut *tx).await.map_err(repo_err)?;
        sqlx::query("UPDATE processing_stage_runs SET is_current = TRUE, updated_at = ? WHERE document_id = ? AND group_id = ? AND stage = 'embedding' AND state = 'pending' AND attempt = 0 AND job_id IS NULL AND vault_id = ? AND revision_id IS ?")
            .bind(&now).bind(fence.document_id.to_string()).bind(fence.group_id.to_string()).bind(fence.vault_id.to_string()).bind(fence.revision_id.map(|id| id.to_string())).execute(&mut *tx).await.map_err(repo_err)?;
        owns_embedding = true;
    }
    sqlx::query(
        "UPDATE documents SET content = ?, ocr_status = ?, embed_status = COALESCE(?, embed_status), updated_at = ? WHERE id = ?",
    )
    .bind(&content)
    .bind(if completed { "done" } else { "skipped" })
    .bind(owns_embedding.then_some(if embed { "pending" } else { "skipped" }))
    .bind(&now)
    .bind(fence.document_id.to_string())
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
        insert_domain_event_with_origin(&mut tx, &event, stage.origin).await?;
    }
    let plan = if embed {
        None
    } else if has_text {
        Some(ProcessingReason::ProviderDisabled)
    } else {
        Some(ProcessingReason::NoText)
    };
    if !live_claim(&mut tx, fence).await? {
        return Ok(false);
    }
    if owns_embedding {
        complete_embedding_plan(&mut tx, fence, &snapshot, stage.lineage, plan).await?;
    }
    if !live_claim(&mut tx, fence).await? {
        return Ok(false);
    }
    tx.commit().await.map_err(repo_err)?;
    Ok(true)
}

async fn complete_embedding_plan(
    conn: &mut SqliteConnection,
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
                    insert_job_sqlite(
                        conn,
                        fence.embedding_job(),
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
    let run_id: Option<String> = sqlx::query_scalar(
        "UPDATE processing_stage_runs SET state = 'skipped', attempt = 1, queue_attempt = ?,
             job_id = COALESCE(job_id, ?), claim_id = NULL, reason_code = ?,
             retry_at = NULL, updated_at = ?
         WHERE group_id = ? AND document_id = ? AND vault_id = ? AND stage = 'embedding'
           AND state = 'pending' AND attempt = 0 AND is_current = 1 AND revision_id IS ?
           AND EXISTS (SELECT 1 FROM jobs WHERE id = ? AND claim_id = ? AND attempts = ?
               AND status = 'processing' AND lease_expires_at > CAST(strftime('%s', 'now') AS INTEGER))
         RETURNING id",
    )
    .bind(fence.queue_attempt)
    .bind(fence.job_id.to_string())
    .bind(reason_code(reason)?)
    .bind(Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true))
    .bind(fence.group_id.to_string())
    .bind(fence.document_id.to_string())
    .bind(fence.vault_id.to_string())
    .bind(fence.revision_id.map(|id| id.to_string()))
    .bind(fence.job_id.to_string())
    .bind(fence.claim_id.to_string())
    .bind(fence.queue_attempt)
    .fetch_optional(&mut *conn)
    .await
    .map_err(repo_err)?;
    let run_id = run_id
        .ok_or_else(|| Error::Conflict("Pending embedding run or OCR lease changed".into()))?;
    let event = embedding_skip_envelope(
        snapshot,
        run_id.parse().map_err(repo_err)?,
        fence.revision_id,
        lineage,
        reason,
    )
    .await?;
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

/// Commit validated chunks, completion status, and their canonical/compatibility event atomically.
pub async fn complete_embedding(
    pool: &SqlitePool,
    fence: &super::StageFence,
    expected_content: &str,
    model: &str,
    chunks: &[folio_core::ports::EmbeddedChunk],
) -> Result<bool> {
    if fence.stage != super::ProcessingEventStage::Embedding {
        return Ok(false);
    }
    finish_derived(
        pool,
        fence,
        super::DerivedResult::Embedding {
            expected_content,
            model,
            chunks,
        },
    )
    .await
}

/// Resolve a disabled or unsupported derived stage without publishing a result.
pub async fn skip_derived(
    pool: &SqlitePool,
    fence: &super::StageFence,
    reason: ProcessingReason,
) -> Result<bool> {
    let valid = match reason {
        ProcessingReason::ProviderDisabled => true,
        ProcessingReason::NoText => fence.stage == super::ProcessingEventStage::Embedding,
        ProcessingReason::UnsupportedFormat => {
            fence.stage == super::ProcessingEventStage::Thumbnail
        }
        _ => false,
    };
    if !valid || fence.stage == super::ProcessingEventStage::Ocr {
        return Ok(false);
    }
    finish_derived(pool, fence, super::DerivedResult::Skipped(reason)).await
}

/// Publish immutable thumbnail objects uploaded beneath the fence's result prefix.
pub async fn complete_thumbnail(
    pool: &SqlitePool,
    fence: &super::StageFence,
    page_count: Option<u32>,
) -> Result<bool> {
    if fence.stage != super::ProcessingEventStage::Thumbnail {
        return Ok(false);
    }
    finish_derived(pool, fence, super::DerivedResult::Thumbnail { page_count }).await
}

async fn finish_derived(
    pool: &SqlitePool,
    fence: &super::StageFence,
    result: super::DerivedResult<'_>,
) -> Result<bool> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(repo_err)?;
    let Some(stage) = load(
        &mut tx,
        fence.document_id,
        fence.job_id,
        fence.job_type,
        fence.claim_id,
        fence.queue_attempt,
        fence.stage,
    )
    .await?
    else {
        return Ok(false);
    };
    let skipped = matches!(result, super::DerivedResult::Skipped(_));
    if !stage.accepts_finish(
        fence,
        if skipped {
            AttemptOutcome::Skipped
        } else {
            AttemptOutcome::Completed
        },
    ) {
        return Ok(false);
    }
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true);
    let mut prefix = None;
    let mut pages = None;
    match &result {
        super::DerivedResult::Embedding {
            expected_content,
            model,
            chunks,
        } => {
            let current_content: Option<String> =
                sqlx::query_scalar("SELECT content FROM documents WHERE id = ?")
                    .bind(fence.document_id.to_string())
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(repo_err)?;
            if current_content.as_deref().unwrap_or_default() != *expected_content {
                return Err(Error::Conflict("Embedding source content changed".into()));
            }
            super::validate_chunks(model, chunks)?;
            sqlx::query("DELETE FROM document_chunks WHERE document_id = ?")
                .bind(fence.document_id.to_string())
                .execute(&mut *tx)
                .await
                .map_err(repo_err)?;
            for chunk in *chunks {
                let query = sqlx::query("INSERT INTO document_chunks (document_id, chunk_index, text, model, dimensions, embedding, stale, created_at, revision_id) VALUES (?,?,?,?,?,?,0,?,?)");
                let query = query
                    .bind(fence.document_id.to_string())
                    .bind(i64::from(chunk.index))
                    .bind(&chunk.text)
                    .bind(*model)
                    .bind(chunk.embedding.len() as i64)
                    .bind(
                        chunk
                            .embedding
                            .iter()
                            .flat_map(|f| f.to_le_bytes())
                            .collect::<Vec<u8>>(),
                    )
                    .bind(&now)
                    .bind(fence.revision_id.map(|id| id.to_string()));

                query.execute(&mut *tx).await.map_err(repo_err)?;
            }
        }
        super::DerivedResult::Thumbnail { page_count } => {
            prefix = Some(fence.result_prefix());
            pages = *page_count;
        }
        super::DerivedResult::Skipped(ProcessingReason::NoText) => {
            let current_content: Option<String> =
                sqlx::query_scalar("SELECT content FROM documents WHERE id = ?")
                    .bind(fence.document_id.to_string())
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(repo_err)?;
            if current_content
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty())
            {
                return Err(Error::Conflict("Embedding source content changed".into()));
            }
            sqlx::query("DELETE FROM document_chunks WHERE document_id = ?")
                .bind(fence.document_id.to_string())
                .execute(&mut *tx)
                .await
                .map_err(repo_err)?;
        }
        super::DerivedResult::Skipped(_) => {}
    }
    let reason = match result {
        super::DerivedResult::Skipped(reason) => Some(reason),
        _ => None,
    };
    let changed = sqlx::query("UPDATE processing_stage_runs SET state = ?, claim_id = NULL, reason_code = ?, retry_at = NULL, result_prefix = ?, updated_at = ? WHERE id = ? AND EXISTS (SELECT 1 FROM jobs WHERE id = ? AND claim_id = ? AND attempts = ? AND status = 'processing' AND lease_expires_at > unixepoch())")
        .bind(if skipped { "skipped" } else { "completed" }).bind(reason.map(reason_code).transpose()?).bind(prefix).bind(&now).bind(fence.run_id.to_string()).bind(fence.job_id.to_string()).bind(fence.claim_id.to_string()).bind(fence.queue_attempt)
        .execute(&mut *tx).await.map_err(repo_err)?;
    if changed.rows_affected() != 1 {
        return Ok(false);
    }
    sqlx::query("UPDATE documents SET embed_status = CASE WHEN ? = 'embedding' THEN ? ELSE embed_status END, page_count = COALESCE(?, page_count), updated_at = ? WHERE id = ?")
        .bind(super::stage_name(fence.stage)).bind(if skipped { "skipped" } else { "done" }).bind(pages.map(i64::from)).bind(&now).bind(fence.document_id.to_string())
        .execute(&mut *tx).await.map_err(repo_err)?;
    let snapshot = capture_document(&mut tx, fence.document_id, Some(fence.vault_id))
        .await?
        .ok_or_else(|| repo_err("Fenced processing document disappeared"))?;
    let outcome = match reason {
        Some(reason) => ProcessingEventOutcome::Skipped { reason },
        None => ProcessingEventOutcome::Completed { has_text: None },
    };
    let event = envelope(&snapshot, fence, stage.lineage, outcome).await?;
    let projection = match fence.stage {
        super::ProcessingEventStage::Embedding => Some(DocumentEvent::Processed {
            document_id: fence.document_id,
        }),
        super::ProcessingEventStage::Thumbnail if !skipped => Some(DocumentEvent::ThumbnailReady {
            document_id: fence.document_id,
        }),
        _ => None,
    };
    if let Some(projection) = projection {
        insert_event_with_envelope(&mut tx, &projection, stage.origin, Some(&snapshot), &event)
            .await?;
    } else {
        insert_domain_event_with_origin(&mut tx, &event, stage.origin).await?;
    }
    if !live_claim(&mut tx, fence).await? {
        return Ok(false);
    }
    tx.commit().await.map_err(repo_err)?;
    Ok(true)
}

async fn live_claim(conn: &mut SqliteConnection, fence: &super::StageFence) -> Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM jobs WHERE id = ? AND claim_id = ? AND attempts = ? AND status = 'processing' AND lease_expires_at > unixepoch())")
        .bind(fence.job_id.to_string()).bind(fence.claim_id.to_string()).bind(fence.queue_attempt).fetch_one(conn).await.map_err(repo_err)
}
