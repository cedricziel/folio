use chrono::{DateTime, Utc};
use folio_core::processing_events::ProcessingReason;
use folio_core::Result;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

use super::{repo_err, Failure, JobRow, StageRow};
use crate::database::sqlite::{capture_document, insert_domain_event};
use crate::queue::{FailOutcome, Job};

async fn load(conn: &mut SqliteConnection, id: Uuid) -> Result<Option<JobRow>> {
    sqlx::query_as(
        "SELECT job_type, payload, status, attempts, max_attempts, claim_id,
        COALESCE(lease_expires_at > unixepoch(), FALSE) AS live FROM jobs WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(conn)
    .await
    .map_err(repo_err)
}

async fn stages(conn: &mut SqliteConnection, id: Uuid) -> Result<Vec<StageRow>> {
    sqlx::query_as("SELECT r.id, r.document_id, r.vault_id, r.stage, r.attempt,
               j.correlation_id, j.causation_id
        FROM processing_stage_runs r JOIN jobs j ON j.id = r.job_id
        JOIN documents d ON d.id = r.document_id
        WHERE j.id = ? AND r.group_id = j.processing_group_id AND r.vault_id = j.origin_vault_id
          AND d.vault_id = r.vault_id AND d.deleted_at IS NULL AND d.purge_requested_at IS NULL
          AND r.state = 'running' AND r.is_current = TRUE AND r.revision_id IS NULL
          AND r.claim_id = j.claim_id AND r.queue_attempt = j.attempts
          AND j.correlation_id IS NOT NULL
          AND json_extract(j.payload, '$.data.document_id') = d.id
          AND json_extract(j.payload, '$.type') = CASE j.job_type
              WHEN 'process_document' THEN 'ProcessDocument' WHEN 'extract_text' THEN 'ExtractText'
              WHEN 'generate_embedding' THEN 'GenerateEmbedding' WHEN 'generate_thumbnail' THEN 'GenerateThumbnail' END")
        .bind(id.to_string()).fetch_all(conn).await.map_err(repo_err)
}

async fn settle(pool: &SqlitePool, id: Uuid, failure: Failure<'_>) -> Result<Option<FailOutcome>> {
    let preliminary = {
        let mut conn = pool.acquire().await.map_err(repo_err)?;
        load(&mut conn, id).await?
    };
    let Some(preliminary) = preliminary else {
        return Ok(None);
    };
    let document_id = preliminary.document_id();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(repo_err)?;
    let Some(row) = load(&mut tx, id).await? else {
        return Ok(None);
    };
    if row.document_id() != document_id {
        return Ok(None);
    }
    let now: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
    let now: DateTime<Utc> = now.parse().map_err(repo_err)?;
    let Some(decision) = failure.decision(&row, now)? else {
        return Ok(None);
    };
    let running = stages(&mut tx, id).await?;
    let changed = sqlx::query(
        "UPDATE jobs SET status = ?, error = ?, scheduled_at = COALESCE(?, scheduled_at),
            completed_at = ?, updated_at = ?, claim_id = NULL, lease_expires_at = NULL
        WHERE id = ? AND attempts = ? AND claim_id IS ? AND status = 'processing'
          AND ((? AND (lease_expires_at IS NULL OR lease_expires_at <= unixepoch()))
            OR (NOT ? AND lease_expires_at > unixepoch()))",
    )
    .bind(if decision.retry_at.is_some() {
        "pending"
    } else {
        "failed"
    })
    .bind(&decision.error)
    .bind(decision.retry_at.map(|at| at.to_rfc3339()))
    .bind(decision.retry_at.is_none().then(|| now.to_rfc3339()))
    .bind(now.to_rfc3339())
    .bind(id.to_string())
    .bind(row.attempts)
    .bind(row.claim_id)
    .bind(failure.expired())
    .bind(failure.expired())
    .execute(&mut *tx)
    .await
    .map_err(repo_err)?;
    if changed.rows_affected() != 1 {
        return Ok(None);
    }
    for stage in running {
        stage.occurrence(&decision)?;
        sqlx::query("UPDATE processing_stage_runs SET state = ?, reason_code = ?, retry_at = ?, claim_id = NULL, updated_at = ? WHERE id = ?")
            .bind(decision.stage_state()).bind(decision.reason_code()?)
            .bind(decision.retry_at.map(|at| at.to_rfc3339())).bind(now.to_rfc3339()).bind(&stage.id)
            .execute(&mut *tx).await.map_err(repo_err)?;
        sqlx::query("UPDATE documents SET ocr_status = CASE WHEN ? = 'ocr' THEN 'failed' ELSE ocr_status END,
                embed_status = CASE WHEN ? = 'embedding' THEN 'failed' ELSE embed_status END,
                updated_at = ? WHERE id = ?")
            .bind(&stage.stage).bind(&stage.stage).bind(now.to_rfc3339()).bind(&stage.document_id)
            .execute(&mut *tx).await.map_err(repo_err)?;
        let snapshot = capture_document(&mut tx, stage.document()?, Some(stage.vault()?))
            .await?
            .ok_or_else(|| repo_err("Fenced processing document disappeared"))?;
        let envelope = stage.envelope(&snapshot, &decision).await?;
        insert_domain_event(&mut tx, &envelope).await?;
    }
    tx.commit().await.map_err(repo_err)?;
    Ok(Some(decision.outcome()))
}

pub(in crate::queue) async fn fail(
    pool: &SqlitePool,
    job: &Job,
    error: &str,
    retry: bool,
    reason: ProcessingReason,
) -> Result<FailOutcome> {
    Ok(settle(
        pool,
        job.id,
        Failure::Report {
            job,
            error,
            retry,
            reason,
        },
    )
    .await?
    .unwrap_or(FailOutcome::LostLease))
}

pub(in crate::queue) async fn reset_stuck(pool: &SqlitePool) -> Result<i64> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM jobs WHERE status = 'processing' AND (lease_expires_at IS NULL OR lease_expires_at <= unixepoch())")
        .fetch_all(pool).await.map_err(repo_err)?;
    let mut recovered = 0;
    for id in ids {
        let settled = match id.parse() {
            Ok(id) => settle(pool, id, Failure::Expired).await,
            Err(error) => Err(repo_err(error)),
        };
        match settled {
            Ok(Some(_)) => recovered += 1,
            Ok(None) => {}
            Err(error) => tracing::error!(job.id = %id, %error, "Failed to recover expired job"),
        }
    }
    if recovered > 0 {
        tracing::info!("Recovered {recovered} expired jobs");
    }
    Ok(recovered)
}
