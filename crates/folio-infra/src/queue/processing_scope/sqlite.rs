use chrono::Utc;
use folio_core::{models::VaultId, Result};
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

use super::{
    assigned_stage, child_scope, repo_err, stages, stale_parent, ClaimPreparation, EnqueuePlan,
    ProcessingScope,
};
use crate::queue::{Job, JobType};

async fn document(conn: &mut SqliteConnection, id: Uuid) -> Result<Option<(String, bool)>> {
    sqlx::query_as("SELECT vault_id, (deleted_at IS NOT NULL OR purge_requested_at IS NOT NULL) FROM documents WHERE id = ?")
        .bind(id.to_string())
        .fetch_optional(conn)
        .await
        .map_err(repo_err)
}

async fn valid_scope(conn: &mut SqliteConnection, scope: ProcessingScope) -> Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM jobs j JOIN documents d ON d.id = ?
         WHERE j.id = ? AND j.claim_id = ? AND j.attempts = ? AND j.status = 'processing'
           AND j.lease_expires_at > unixepoch() AND j.processing_group_id = ?
           AND j.origin_vault_id = ? AND d.vault_id = j.origin_vault_id AND d.deleted_at IS NULL AND d.purge_requested_at IS NULL
           AND json_extract(j.payload, '$.data.document_id') = d.id
           AND json_extract(j.payload, '$.type') = CASE j.job_type
               WHEN 'process_document' THEN 'ProcessDocument' WHEN 'extract_text' THEN 'ExtractText'
               WHEN 'generate_embedding' THEN 'GenerateEmbedding' WHEN 'generate_thumbnail' THEN 'GenerateThumbnail' END
           AND EXISTS (SELECT 1 FROM processing_stage_runs r WHERE r.group_id = j.processing_group_id
               AND r.job_id = j.id AND r.document_id = d.id AND r.vault_id = d.vault_id
               AND r.is_current = 1 AND r.revision_id IS NULL)
         )")
        .bind(scope.document_id.to_string()).bind(scope.parent_job_id.to_string())
        .bind(scope.claim_id.to_string()).bind(scope.queue_attempt)
        .bind(scope.group_id.to_string()).bind(scope.vault_id.to_string())
        .fetch_one(conn).await.map_err(repo_err)
}

pub(in crate::queue) async fn prepare_enqueue(
    conn: &mut SqliteConnection,
    job: &JobType,
    id: Uuid,
) -> Result<EnqueuePlan> {
    let doc = match job.document_id() {
        Some(id) => document(conn, id).await?,
        None => None,
    };
    let vault_id = doc
        .as_ref()
        .map(|(id, _)| id.parse())
        .transpose()
        .map_err(repo_err)?;
    if let Some(scope) = child_scope(job) {
        if !valid_scope(conn, scope).await? {
            return Err(stale_parent());
        }
        let stage = assigned_stage(job).ok_or_else(stale_parent)?;
        let row: Option<(Option<String>, String)> = sqlx::query_as(
            "SELECT job_id, state FROM processing_stage_runs WHERE group_id = ? AND stage = ?
             AND document_id = ? AND vault_id = ? AND is_current = 1 AND revision_id IS NULL",
        )
        .bind(scope.group_id.to_string())
        .bind(stage)
        .bind(scope.document_id.to_string())
        .bind(scope.vault_id.to_string())
        .fetch_optional(&mut *conn)
        .await
        .map_err(repo_err)?;
        let Some((assigned, state)) = row else {
            return Err(stale_parent());
        };
        let existing = assigned.is_some();
        if !existing && state != "pending" {
            return Err(stale_parent());
        }
        return Ok(EnqueuePlan {
            id: assigned
                .map(|id| id.parse())
                .transpose()
                .map_err(repo_err)?
                .unwrap_or(id),
            vault_id: Some(scope.vault_id.0),
            group_id: Some(scope.group_id),
            existing,
            child_stage: Some(stage),
        });
    }
    Ok(EnqueuePlan {
        id,
        vault_id,
        group_id: doc
            .filter(|(_, deleted)| !deleted)
            .filter(|_| !stages(job).is_empty())
            .map(|_| id),
        existing: false,
        child_stage: None,
    })
}

async fn seed(
    conn: &mut SqliteConnection,
    job: &JobType,
    job_id: Uuid,
    group_id: Uuid,
    vault_id: Uuid,
) -> Result<()> {
    let document_id = job.document_id().expect("stage jobs have a document");
    let now = Utc::now().to_rfc3339();
    for stage in stages(job) {
        sqlx::query("UPDATE processing_stage_runs SET is_current = 0, updated_at = ? WHERE document_id = ? AND stage = ? AND is_current = 1")
            .bind(&now).bind(document_id.to_string()).bind(stage).execute(&mut *conn).await.map_err(repo_err)?;
        sqlx::query("INSERT INTO processing_stage_runs (id, document_id, vault_id, job_id, group_id, stage, state, attempt, queue_attempt, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, 'pending', 0, 0, ?, ?)")
            .bind(Uuid::new_v4().to_string()).bind(document_id.to_string()).bind(vault_id.to_string())
            .bind((Some(*stage) == assigned_stage(job)).then(|| job_id.to_string()))
            .bind(group_id.to_string()).bind(stage).bind(&now).bind(&now)
            .execute(&mut *conn).await.map_err(repo_err)?;
    }
    Ok(())
}

pub(in crate::queue) async fn finish_enqueue(
    conn: &mut SqliteConnection,
    job: &JobType,
    plan: &EnqueuePlan,
) -> Result<()> {
    let (Some(group), Some(vault)) = (plan.group_id, plan.vault_id) else {
        return Ok(());
    };
    if let Some(stage) = plan.child_stage {
        let changed = sqlx::query("UPDATE processing_stage_runs SET job_id = ?, updated_at = ? WHERE group_id = ? AND stage = ? AND job_id IS NULL AND state = 'pending' AND is_current = 1")
            .bind(plan.id.to_string()).bind(Utc::now().to_rfc3339()).bind(group.to_string()).bind(stage)
            .execute(conn).await.map_err(repo_err)?;
        if changed.rows_affected() != 1 {
            return Err(stale_parent());
        }
        Ok(())
    } else {
        seed(conn, job, plan.id, group, vault).await
    }
}

pub(super) async fn prepare_claim(pool: &SqlitePool, job: &Job) -> Result<ClaimPreparation> {
    let Some(document_id) = job.job_type.document_id() else {
        return Ok(ClaimPreparation::Ready(None));
    };
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(repo_err)?;
    let Some((vault, deleted)) = document(&mut tx, document_id).await? else {
        return Ok(ClaimPreparation::Ready(None));
    };
    if deleted {
        return Ok(ClaimPreparation::Obsolete);
    }
    let row: Option<(Option<String>, Option<String>, Option<String>, String)> = sqlx::query_as(
        "SELECT processing_group_id, origin_vault_id, correlation_id, created_at FROM jobs
         WHERE id = ? AND claim_id = ? AND attempts = ? AND status = 'processing' AND lease_expires_at > unixepoch() AND job_type = ?
           AND json_extract(payload, '$.type') = ? AND json_extract(payload, '$.data.document_id') = ?")
        .bind(job.id.to_string()).bind(job.claim_id.map(|id| id.to_string())).bind(job.attempts)
        .bind(job.job_type.type_name()).bind(super::payload_type(&job.job_type)?)
        .bind(document_id.to_string())
        .fetch_optional(&mut *tx).await.map_err(repo_err)?;
    let Some((group, origin, correlation, created_at)) = row else {
        return Ok(ClaimPreparation::Obsolete);
    };
    if origin.as_ref().is_some_and(|origin| origin != &vault) {
        return Ok(ClaimPreparation::Obsolete);
    }
    let scope = ProcessingScope {
        group_id: group
            .as_ref()
            .map(|id| id.parse())
            .transpose()
            .map_err(repo_err)?
            .unwrap_or(job.id),
        document_id,
        vault_id: VaultId(vault.parse().map_err(repo_err)?),
        parent_job_id: job.id,
        claim_id: job.claim_id.ok_or_else(stale_parent)?,
        queue_attempt: job.attempts,
    };
    if group.is_some() {
        if !valid_scope(&mut tx, scope).await? {
            return Ok(ClaimPreparation::Obsolete);
        }
    } else {
        for stage in stages(&job.job_type) {
            let newer: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM processing_stage_runs WHERE document_id = ? AND stage = ? AND is_current = 1 AND julianday(created_at) >= julianday(?))")
                .bind(document_id.to_string()).bind(stage).bind(&created_at).fetch_one(&mut *tx).await.map_err(repo_err)?;
            if newer {
                return Ok(ClaimPreparation::Obsolete);
            }
        }
        let correlation = correlation.unwrap_or_else(|| Uuid::new_v4().to_string());
        sqlx::query("UPDATE jobs SET processing_group_id = ?, origin_vault_id = COALESCE(origin_vault_id, ?), correlation_id = COALESCE(correlation_id, ?) WHERE id = ?")
            .bind(scope.group_id.to_string()).bind(&vault).bind(correlation).bind(job.id.to_string())
            .execute(&mut *tx).await.map_err(repo_err)?;
        seed(
            &mut tx,
            &job.job_type,
            job.id,
            scope.group_id,
            scope.vault_id.0,
        )
        .await?;
    }
    tx.commit().await.map_err(repo_err)?;
    Ok(ClaimPreparation::Ready(Some(scope)))
}
