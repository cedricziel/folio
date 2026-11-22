use chrono::Utc;
use folio_core::models::AttemptOutcome;
use folio_core::mutation::{with_actor, with_lineage, MutationLineage};
use folio_core::processing_events::{ProcessingEventOutcome, ProcessingReason};
use folio_core::{DocumentEvent, Error, EventOrigin, EventSnapshot, Result};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::database::postgres::{
    capture_document, insert_domain_event_with_origin, insert_event_with_envelope,
};
use crate::queue::processing_scope::with_scope;
use crate::queue::{insert_job_postgres, Job, DEFAULT_MAX_ATTEMPTS};

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
    stage: super::ProcessingEventStage,
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
    let row = sqlx::query_as::<_, StageRow>(
        "SELECT r.id::text AS run_id, r.vault_id::text, r.group_id::text, r.stage, r.state, r.attempt, j.payload::text AS payload,
                r.revision_id::text, r.claim_id::text AS stage_claim_id, r.queue_attempt AS stage_queue_attempt,
                j.correlation_id::text, j.causation_id::text
         FROM documents d JOIN jobs j ON j.id = $1
         JOIN processing_stage_runs r ON r.document_id = d.id AND r.job_id = j.id
         JOIN vaults v ON v.id = d.vault_id AND v.deleted_at IS NULL
         WHERE d.id = $2 AND d.deleted_at IS NULL AND d.purge_requested_at IS NULL
           AND d.vault_id = j.origin_vault_id
           AND r.vault_id = j.origin_vault_id AND r.group_id = j.processing_group_id
           AND r.stage = $6 AND r.is_current
           AND r.revision_id::text IS NOT DISTINCT FROM j.payload->'data'->>'revision_id'
           AND ((r.revision_id IS NULL AND d.current_revision_id IS NULL AND NOT EXISTS (
               SELECT 1 FROM document_revisions history WHERE history.document_id = d.id
           )) OR EXISTS (
               SELECT 1 FROM document_revisions revision
               WHERE revision.id = d.current_revision_id AND revision.document_id = d.id
                 AND revision.vault_id = d.vault_id
                 AND (revision.id = r.revision_id OR (r.revision_id IS NULL AND revision.number = 1))
           ))
           AND j.status = 'processing' AND j.claim_id = $3 AND j.attempts = $4
           AND j.lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint
           AND j.correlation_id IS NOT NULL
           AND j.job_type = $5 AND j.payload->>'type' = CASE j.job_type
               WHEN 'process_document' THEN 'ProcessDocument' WHEN 'extract_text' THEN 'ExtractText'
               WHEN 'generate_embedding' THEN 'GenerateEmbedding' WHEN 'generate_thumbnail' THEN 'GenerateThumbnail'
               WHEN 'process_document_revision' THEN 'ProcessDocumentRevision'
               WHEN 'extract_revision_text' THEN 'ExtractRevisionText'
               WHEN 'generate_revision_embedding' THEN 'GenerateRevisionEmbedding'
               WHEN 'generate_revision_thumbnail' THEN 'GenerateRevisionThumbnail' END
           AND j.payload->'data'->>'document_id' = d.id::text
         FOR UPDATE OF r FOR SHARE OF v",
    )
    .bind(job_id)
    .bind(document_id)
    .bind(claim_id)
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
pub async fn start_ocr(pool: &PgPool, job: &Job) -> Result<Option<OcrStageFence>> {
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
    pool: &PgPool,
    job: &Job,
    stage: super::ProcessingEventStage,
) -> Result<Option<super::StageFence>> {
    let Some((document_id, claim_id)) = super::claimed_stage_document(job, stage) else {
        return Ok(None);
    };
    start_stage(pool, job, stage, document_id, claim_id).await
}

async fn start_stage(
    pool: &PgPool,
    job: &Job,
    stage_kind: super::ProcessingEventStage,
    document_id: Uuid,
    claim_id: Uuid,
) -> Result<Option<super::StageFence>> {
    let mut tx = pool.begin().await.map_err(repo_err)?;
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
    sqlx::query("UPDATE documents SET ocr_status = CASE WHEN $3 = 'ocr' THEN 'running' ELSE ocr_status END, embed_status = CASE WHEN $3 = 'embedding' THEN 'running' ELSE embed_status END, updated_at = $1 WHERE id = $2")
        .bind(now)
        .bind(document_id)
        .bind(super::stage_name(stage_kind))
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
    if fence.stage != super::ProcessingEventStage::Ocr {
        return Ok(false);
    }
    let mut tx = pool.begin().await.map_err(repo_err)?;
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
    let now = Utc::now();
    let prior_content: Option<String> =
        sqlx::query_scalar("SELECT content FROM documents WHERE id = $1")
            .bind(fence.document_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(repo_err)?;
    let content = content.map(str::to_owned).or_else(|| prior_content.clone());
    if content != prior_content {
        sqlx::query("DELETE FROM document_chunks WHERE document_id = $1")
            .bind(fence.document_id)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
    }
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
    let mut owns_embedding: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM processing_stage_runs WHERE group_id = $1 AND document_id = $2
             AND vault_id = $3 AND revision_id IS NOT DISTINCT FROM $4
             AND stage = 'embedding' AND is_current)",
    )
    .bind(fence.group_id)
    .bind(fence.document_id)
    .bind(fence.vault_id.0)
    .bind(fence.revision_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(repo_err)?;
    if !owns_embedding && content != prior_content {
        let unassigned: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM processing_stage_runs WHERE group_id = $1 AND document_id = $2 AND vault_id = $3 AND stage = 'embedding' AND state = 'pending' AND attempt = 0 AND job_id IS NULL AND revision_id IS NOT DISTINCT FROM $4)")
            .bind(fence.group_id).bind(fence.document_id).bind(fence.vault_id.0).bind(fence.revision_id)
            .fetch_one(&mut *tx).await.map_err(repo_err)?;
        if !unassigned {
            return Err(Error::Conflict(
                "Embedding plan changed before OCR completion".into(),
            ));
        }
        sqlx::query("UPDATE processing_stage_runs SET is_current = FALSE, updated_at = $1 WHERE document_id = $2 AND stage = 'embedding' AND is_current = TRUE")
            .bind(now).bind(fence.document_id).execute(&mut *tx).await.map_err(repo_err)?;
        sqlx::query("UPDATE processing_stage_runs SET is_current = TRUE, updated_at = $1 WHERE document_id = $2 AND group_id = $3 AND stage = 'embedding' AND state = 'pending' AND attempt = 0 AND job_id IS NULL AND vault_id = $4 AND revision_id IS NOT DISTINCT FROM $5")
            .bind(now).bind(fence.document_id).bind(fence.group_id).bind(fence.vault_id.0).bind(fence.revision_id).execute(&mut *tx).await.map_err(repo_err)?;
        owns_embedding = true;
    }
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
    let run_id: Option<Uuid> = sqlx::query_scalar(
        "UPDATE processing_stage_runs SET state = 'skipped', attempt = 1, queue_attempt = $1,
             job_id = COALESCE(job_id, $2), claim_id = NULL, reason_code = $8,
             retry_at = NULL, updated_at = $3
         WHERE group_id = $4 AND document_id = $5 AND vault_id = $6 AND stage = 'embedding'
           AND state = 'pending' AND attempt = 0 AND is_current AND revision_id IS NOT DISTINCT FROM $9
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
    .bind(fence.revision_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(repo_err)?;
    let run_id = run_id
        .ok_or_else(|| Error::Conflict("Pending embedding run or OCR lease changed".into()))?;
    let event =
        embedding_skip_envelope(snapshot, run_id, fence.revision_id, lineage, reason).await?;
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
    pool: &PgPool,
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
    pool: &PgPool,
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
    pool: &PgPool,
    fence: &super::StageFence,
    page_count: Option<u32>,
) -> Result<bool> {
    if fence.stage != super::ProcessingEventStage::Thumbnail {
        return Ok(false);
    }
    finish_derived(pool, fence, super::DerivedResult::Thumbnail { page_count }).await
}

async fn finish_derived(
    pool: &PgPool,
    fence: &super::StageFence,
    result: super::DerivedResult<'_>,
) -> Result<bool> {
    let mut tx = pool.begin().await.map_err(repo_err)?;
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
    let now = Utc::now();
    let mut prefix = None;
    let mut pages = None;
    match &result {
        super::DerivedResult::Embedding {
            expected_content,
            model,
            chunks,
        } => {
            let current_content: Option<String> =
                sqlx::query_scalar("SELECT content FROM documents WHERE id = $1")
                    .bind(fence.document_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(repo_err)?;
            if current_content.as_deref().unwrap_or_default() != *expected_content {
                return Err(Error::Conflict("Embedding source content changed".into()));
            }
            super::validate_chunks(model, chunks)?;
            sqlx::query("DELETE FROM document_chunks WHERE document_id = $1")
                .bind(fence.document_id)
                .execute(&mut *tx)
                .await
                .map_err(repo_err)?;
            let pgvector: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'document_chunks' AND column_name = 'embedding_vec' AND table_schema = current_schema())").fetch_one(&mut *tx).await.map_err(repo_err)?;
            for chunk in *chunks {
                let query = if pgvector {
                    sqlx::query("INSERT INTO document_chunks (document_id, chunk_index, text, model, dimensions, embedding, stale, created_at, embedding_vec) VALUES ($1,$2,$3,$4,$5,$6,FALSE,$7,$8::vector)")
                } else {
                    sqlx::query("INSERT INTO document_chunks (document_id, chunk_index, text, model, dimensions, embedding, stale, created_at) VALUES ($1,$2,$3,$4,$5,$6,FALSE,$7)")
                };
                let query = query
                    .bind(fence.document_id)
                    .bind(chunk.index as i32)
                    .bind(&chunk.text)
                    .bind(*model)
                    .bind(chunk.embedding.len() as i32)
                    .bind(&chunk.embedding)
                    .bind(now);
                let query = if pgvector {
                    query.bind(format!(
                        "[{}]",
                        chunk
                            .embedding
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(",")
                    ))
                } else {
                    query
                };
                query.execute(&mut *tx).await.map_err(repo_err)?;
            }
        }
        super::DerivedResult::Thumbnail { page_count } => {
            prefix = Some(fence.result_prefix());
            pages = *page_count;
        }
        super::DerivedResult::Skipped(ProcessingReason::NoText) => {
            let current_content: Option<String> =
                sqlx::query_scalar("SELECT content FROM documents WHERE id = $1")
                    .bind(fence.document_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(repo_err)?;
            if current_content
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty())
            {
                return Err(Error::Conflict("Embedding source content changed".into()));
            }
            sqlx::query("DELETE FROM document_chunks WHERE document_id = $1")
                .bind(fence.document_id)
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
    let changed = sqlx::query("UPDATE processing_stage_runs SET state = $1, claim_id = NULL, reason_code = $2, retry_at = NULL, result_prefix = $3, updated_at = $4 WHERE id = $5 AND EXISTS (SELECT 1 FROM jobs WHERE id = $6 AND claim_id = $7 AND attempts = $8 AND status = 'processing' AND lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint)")
        .bind(if skipped { "skipped" } else { "completed" }).bind(reason.map(reason_code).transpose()?).bind(prefix).bind(now).bind(fence.run_id).bind(fence.job_id).bind(fence.claim_id).bind(fence.queue_attempt)
        .execute(&mut *tx).await.map_err(repo_err)?;
    if changed.rows_affected() != 1 {
        return Ok(false);
    }
    sqlx::query("UPDATE documents SET embed_status = CASE WHEN $1 = 'embedding' THEN $2 ELSE embed_status END, page_count = COALESCE($3, page_count), updated_at = $4 WHERE id = $5")
        .bind(super::stage_name(fence.stage)).bind(if skipped { "skipped" } else { "done" }).bind(pages.map(|n| n as i32)).bind(now).bind(fence.document_id)
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

async fn live_claim(conn: &mut PgConnection, fence: &super::StageFence) -> Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM jobs WHERE id = $1 AND claim_id = $2 AND attempts = $3 AND status = 'processing' AND lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint)")
        .bind(fence.job_id).bind(fence.claim_id).bind(fence.queue_attempt).fetch_one(conn).await.map_err(repo_err)
}
