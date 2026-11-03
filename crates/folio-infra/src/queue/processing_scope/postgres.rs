use chrono::Utc;
use folio_core::{models::VaultId, Result};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{
    assigned_stage, child_scope, repo_err, stages, stale_parent, ClaimPreparation, EnqueuePlan,
    ProcessingScope,
};
use crate::queue::{Job, JobType};

/// Group, origin vault, correlation, and creation time of a claimed job.
type ClaimRow = (Option<String>, Option<String>, Option<String>, String);

async fn document(conn: &mut PgConnection, id: Uuid) -> Result<Option<(String, bool)>> {
    sqlx::query_as("SELECT vault_id::text, (deleted_at IS NOT NULL OR purge_requested_at IS NOT NULL) FROM documents WHERE id = $1::uuid FOR UPDATE")
        .bind(id.to_string()).fetch_optional(conn).await.map_err(repo_err)
}

async fn lock_job(conn: &mut PgConnection, id: Uuid) -> Result<()> {
    sqlx::query("SELECT id FROM jobs WHERE id = $1::uuid FOR UPDATE")
        .bind(id.to_string())
        .execute(conn)
        .await
        .map_err(repo_err)?;
    Ok(())
}

async fn valid_scope(conn: &mut PgConnection, scope: ProcessingScope) -> Result<bool> {
    lock_job(conn, scope.parent_job_id).await?;
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM jobs j JOIN documents d ON d.id = $1::uuid
         WHERE j.id = $2::uuid AND j.claim_id = $3::uuid AND j.attempts = $4 AND j.status = 'processing'
           AND j.lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint AND j.processing_group_id = $5::uuid
           AND j.origin_vault_id = $6::uuid AND d.vault_id = j.origin_vault_id AND d.deleted_at IS NULL AND d.purge_requested_at IS NULL
           AND j.payload->'data'->>'document_id' = d.id::text
           AND j.payload->>'type' = CASE j.job_type
               WHEN 'process_document' THEN 'ProcessDocument' WHEN 'extract_text' THEN 'ExtractText'
               WHEN 'generate_embedding' THEN 'GenerateEmbedding' WHEN 'generate_thumbnail' THEN 'GenerateThumbnail' END
           AND EXISTS (SELECT 1 FROM processing_stage_runs r WHERE r.group_id = j.processing_group_id
               AND r.job_id = j.id AND r.document_id = d.id AND r.vault_id = d.vault_id
               AND r.is_current = TRUE AND r.revision_id IS NULL)
         )")
        .bind(scope.document_id.to_string()).bind(scope.parent_job_id.to_string())
        .bind(scope.claim_id.to_string()).bind(scope.queue_attempt)
        .bind(scope.group_id.to_string()).bind(scope.vault_id.to_string())
        .fetch_one(conn).await.map_err(repo_err)
}

pub(in crate::queue) async fn prepare_enqueue(
    conn: &mut PgConnection,
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
            "SELECT job_id::text, state FROM processing_stage_runs WHERE group_id = $1::uuid AND stage = $2
             AND document_id = $3::uuid AND vault_id = $4::uuid AND is_current = TRUE AND revision_id IS NULL")
            .bind(scope.group_id.to_string()).bind(stage).bind(scope.document_id.to_string())
            .bind(scope.vault_id.to_string()).fetch_optional(&mut *conn).await.map_err(repo_err)?;
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
    conn: &mut PgConnection,
    job: &JobType,
    job_id: Uuid,
    group_id: Uuid,
    vault_id: Uuid,
) -> Result<()> {
    let document_id = job.document_id().expect("stage jobs have a document");
    let now = Utc::now().to_rfc3339();
    for stage in stages(job) {
        sqlx::query("UPDATE processing_stage_runs SET is_current = FALSE, updated_at = $1::timestamptz WHERE document_id = $2::uuid AND stage = $3 AND is_current = TRUE")
            .bind(&now).bind(document_id.to_string()).bind(stage).execute(&mut *conn).await.map_err(repo_err)?;
        sqlx::query("INSERT INTO processing_stage_runs (id, document_id, vault_id, job_id, group_id, stage, state, attempt, queue_attempt, created_at, updated_at)
                     VALUES ($1::uuid, $2::uuid, $3::uuid, $4::uuid, $5::uuid, $6, 'pending', 0, 0, $7::timestamptz, $8::timestamptz)")
            .bind(Uuid::new_v4().to_string()).bind(document_id.to_string()).bind(vault_id.to_string())
            .bind((Some(*stage) == assigned_stage(job)).then(|| job_id.to_string()))
            .bind(group_id.to_string()).bind(stage).bind(&now).bind(&now)
            .execute(&mut *conn).await.map_err(repo_err)?;
    }
    Ok(())
}

pub(in crate::queue) async fn finish_enqueue(
    conn: &mut PgConnection,
    job: &JobType,
    plan: &EnqueuePlan,
) -> Result<()> {
    let (Some(group), Some(vault)) = (plan.group_id, plan.vault_id) else {
        return Ok(());
    };
    if let Some(stage) = plan.child_stage {
        let changed = sqlx::query("UPDATE processing_stage_runs SET job_id = $1::uuid, updated_at = $2::timestamptz WHERE group_id = $3::uuid AND stage = $4 AND job_id IS NULL AND state = 'pending' AND is_current = TRUE")
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

pub(super) async fn prepare_claim(pool: &PgPool, job: &Job) -> Result<ClaimPreparation> {
    let Some(document_id) = job.job_type.document_id() else {
        return Ok(ClaimPreparation::Ready(None));
    };
    let mut tx = pool.begin().await.map_err(repo_err)?;
    let Some((vault, deleted)) = document(&mut tx, document_id).await? else {
        return Ok(ClaimPreparation::Ready(None));
    };
    if deleted {
        return Ok(ClaimPreparation::Obsolete);
    }
    lock_job(&mut tx, job.id).await?;
    let row: Option<ClaimRow> = sqlx::query_as(
        "SELECT processing_group_id::text, origin_vault_id::text, correlation_id::text, created_at::text FROM jobs
         WHERE id = $1::uuid AND claim_id = $2::uuid AND attempts = $3 AND status = 'processing' AND lease_expires_at > floor(extract(epoch FROM clock_timestamp()))::bigint AND job_type = $4
           AND payload->>'type' = $5 AND payload->'data'->>'document_id' = $6")
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
                "SELECT EXISTS (SELECT 1 FROM processing_stage_runs WHERE document_id = $1::uuid AND stage = $2 AND is_current = TRUE AND created_at >= $3::timestamptz)")
                .bind(document_id.to_string()).bind(stage).bind(&created_at).fetch_one(&mut *tx).await.map_err(repo_err)?;
            if newer {
                return Ok(ClaimPreparation::Obsolete);
            }
        }
        let correlation = correlation.unwrap_or_else(|| Uuid::new_v4().to_string());
        sqlx::query("UPDATE jobs SET processing_group_id = $1::uuid, origin_vault_id = COALESCE(origin_vault_id, $2::uuid), correlation_id = COALESCE(correlation_id, $3::uuid) WHERE id = $4::uuid")
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
