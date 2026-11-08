//! PostgreSQL workflow repository implementation

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::instrument;

use super::insert_domain_event;
use crate::database::workflow_lifecycle::workflow_event;
use folio_core::error::{Error, Result};
use folio_core::models::{
    Id, RunStatus, UserId, VaultId, Workflow, WorkflowInput, WorkflowRun, WorkflowTrigger,
};
use folio_core::ports::WorkflowRepository;

use crate::queue::{insert_job_postgres as insert_job, JobType};
use crate::workflow::MAX_ATTEMPTS;

/// PostgreSQL-backed workflow repository
#[derive(Clone)]
pub struct PostgresWorkflowRepository {
    pool: PgPool,
}

impl PostgresWorkflowRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn repo_err(e: impl std::fmt::Display) -> Error {
    Error::Repository(e.to_string())
}

fn parse_trigger(value: &str) -> Result<WorkflowTrigger> {
    WorkflowTrigger::parse(value).ok_or_else(|| repo_err(format!("Unknown trigger '{value}'")))
}

#[derive(sqlx::FromRow)]
struct WorkflowRow {
    id: Id,
    vault_id: Id,
    name: String,
    enabled: bool,
    trigger_event: String,
    conditions: String,
    actions: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
    created_by: Option<String>,
    updated_by: Option<String>,
}

impl TryFrom<WorkflowRow> for Workflow {
    type Error = Error;

    fn try_from(row: WorkflowRow) -> Result<Self> {
        Ok(Workflow {
            id: row.id,
            vault_id: VaultId(row.vault_id),
            name: row.name,
            enabled: row.enabled,
            trigger: parse_trigger(&row.trigger_event)?,
            conditions: serde_json::from_str(&row.conditions).map_err(repo_err)?,
            actions: serde_json::from_str(&row.actions).map_err(repo_err)?,
            created_at: row.created_at,
            updated_at: row.updated_at,
            deleted_at: row.deleted_at,
            created_by: row.created_by.map(UserId),
            updated_by: row.updated_by.map(UserId),
        })
    }
}

#[derive(sqlx::FromRow)]
struct RunRow {
    id: Id,
    vault_id: Id,
    workflow_id: Id,
    event: String,
    document_id: Option<Id>,
    status: String,
    attempts: i32,
    completed_actions: i32,
    last_error: Option<String>,
    response_status: Option<i32>,
    payload: String,
    actions: String,
    source_event_id: Option<i64>,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

impl TryFrom<RunRow> for WorkflowRun {
    type Error = Error;

    fn try_from(row: RunRow) -> Result<Self> {
        Ok(WorkflowRun {
            id: row.id,
            vault_id: VaultId(row.vault_id),
            workflow_id: row.workflow_id,
            event: parse_trigger(&row.event)?,
            document_id: row.document_id,
            status: RunStatus::parse(&row.status),
            attempts: row.attempts as u32,
            completed_actions: row.completed_actions as u32,
            last_error: row.last_error,
            response_status: row.response_status.map(|s| s as u16),
            payload: row.payload,
            actions: serde_json::from_str(&row.actions).map_err(repo_err)?,
            source_event_id: row.source_event_id,
            created_at: row.created_at,
            finished_at: row.finished_at,
        })
    }
}

fn not_found(id: Id) -> Error {
    Error::NotFound {
        entity_type: "Workflow",
        id: id.to_string(),
    }
}

#[async_trait]
impl WorkflowRepository for PostgresWorkflowRepository {
    async fn locate(&self, id: Id) -> Result<Option<VaultId>> {
        let row: Option<(Id,)> =
            sqlx::query_as("SELECT vault_id FROM workflows WHERE id = $1 AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM vaults v WHERE v.id = workflows.vault_id AND v.deleted_at IS NULL)")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
        Ok(row.map(|(v,)| VaultId(v)))
    }

    #[instrument(skip(self))]
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Workflow>> {
        let row: Option<WorkflowRow> = sqlx::query_as(
            "SELECT * FROM workflows WHERE id = $1 AND vault_id = $2 AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM vaults v WHERE v.id = workflows.vault_id AND v.deleted_at IS NULL)",
        )
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(Workflow::try_from).transpose()
    }

    #[instrument(skip(self))]
    async fn list(&self, vault: VaultId) -> Result<Vec<Workflow>> {
        let rows: Vec<WorkflowRow> = sqlx::query_as(
            "SELECT * FROM workflows WHERE vault_id = $1 AND deleted_at IS NULL
             ORDER BY created_at, name",
        )
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(Workflow::try_from).collect()
    }

    #[instrument(skip(self))]
    async fn list_enabled(
        &self,
        vault: VaultId,
        trigger: WorkflowTrigger,
    ) -> Result<Vec<Workflow>> {
        let rows: Vec<WorkflowRow> = sqlx::query_as(
            "SELECT * FROM workflows WHERE vault_id = $1 AND enabled AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM vaults v WHERE v.id = workflows.vault_id AND v.deleted_at IS NULL) AND trigger_event = $2
             ORDER BY created_at",
        )
        .bind(vault.0)
        .bind(trigger.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(Workflow::try_from).collect()
    }

    async fn list_deleted(&self, vault: VaultId) -> Result<Vec<Workflow>> {
        let rows = sqlx::query_as::<_, WorkflowRow>(
            "SELECT * FROM workflows WHERE vault_id = $1 AND deleted_at IS NOT NULL ORDER BY deleted_at DESC, name",
        )
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(Workflow::try_from).collect()
    }

    async fn find_deleted(&self, vault: VaultId, id: Id) -> Result<Option<Workflow>> {
        let row = sqlx::query_as::<_, WorkflowRow>(
            "SELECT * FROM workflows WHERE id = $1 AND vault_id = $2 AND deleted_at IS NOT NULL",
        )
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(Workflow::try_from).transpose()
    }

    #[instrument(skip(self, input))]
    async fn create(&self, vault: VaultId, input: WorkflowInput) -> Result<Workflow> {
        input.validate()?;
        let id = Id::new_v4();
        let now = Utc::now();
        let actor = folio_core::mutation::current_actor();
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row = sqlx::query_as::<_, WorkflowRow>(
            "INSERT INTO workflows (id, vault_id, name, enabled, trigger_event, conditions, actions, created_at, updated_at, created_by, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) RETURNING *",
        )
        .bind(id)
        .bind(vault.0)
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.trigger.as_str())
        .bind(serde_json::to_string(&input.conditions).map_err(repo_err)?)
        .bind(serde_json::to_string(&input.actions).map_err(repo_err)?)
        .bind(now)
        .bind(now)
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(actor.as_ref().map(UserId::as_str))
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        let workflow = Workflow::try_from(row)?;
        insert_domain_event(&mut tx, &workflow_event(&workflow, "created")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(workflow)
    }

    #[instrument(skip(self, input))]
    async fn update(&self, vault: VaultId, id: Id, input: WorkflowInput) -> Result<Workflow> {
        input.validate()?;
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row = sqlx::query_as::<_, WorkflowRow>(
            "SELECT * FROM workflows WHERE id = $1 AND vault_id = $2 AND deleted_at IS NULL FOR UPDATE",
        )
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or_else(|| not_found(id))?;
        let existing = Workflow::try_from(row)?;
        if existing.name == input.name
            && existing.enabled == input.enabled
            && existing.trigger == input.trigger
            && existing.conditions == input.conditions
            && existing.actions == input.actions
        {
            return Ok(existing);
        }
        let actor = folio_core::mutation::current_actor();
        let row = sqlx::query_as::<_, WorkflowRow>(
            "UPDATE workflows SET name = $1, enabled = $2, trigger_event = $3, conditions = $4, actions = $5, updated_at = $6, updated_by = $7 WHERE id = $8 AND vault_id = $9 RETURNING *",
        )
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.trigger.as_str())
        .bind(serde_json::to_string(&input.conditions).map_err(repo_err)?)
        .bind(serde_json::to_string(&input.actions).map_err(repo_err)?)
        .bind(Utc::now())
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id)
        .bind(vault.0)
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        let workflow = Workflow::try_from(row)?;
        insert_domain_event(&mut tx, &workflow_event(&workflow, "updated")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(workflow)
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row = sqlx::query_as::<_, WorkflowRow>(
            "SELECT * FROM workflows WHERE id = $1 AND vault_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or_else(|| not_found(id))?;
        let existing = Workflow::try_from(row)?;
        if existing.deleted_at.is_some() {
            return Ok(());
        }
        let actor = folio_core::mutation::current_actor();
        let now = Utc::now();
        let row = sqlx::query_as::<_, WorkflowRow>(
            "UPDATE workflows SET deleted_at = $1, updated_at = $2, updated_by = $3 WHERE id = $4 AND vault_id = $5 RETURNING *",
        )
        .bind(Some(now))
        .bind(now)
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id)
        .bind(vault.0)
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        let workflow = Workflow::try_from(row)?;
        sqlx::query(
            "UPDATE workflow_runs SET status = 'failed', last_error = 'Workflow was deleted', finished_at = $1 WHERE workflow_id = $2 AND status = 'pending'",
        )
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        insert_domain_event(&mut tx, &workflow_event(&workflow, "deleted")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }

    async fn restore(&self, vault: VaultId, id: Id) -> Result<Workflow> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row = sqlx::query_as::<_, WorkflowRow>(
            "SELECT * FROM workflows WHERE id = $1 AND vault_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or_else(|| not_found(id))?;
        let existing = Workflow::try_from(row)?;
        if existing.deleted_at.is_none() {
            return Ok(existing);
        }
        let actor = folio_core::mutation::current_actor();
        let now = Utc::now();
        let row = sqlx::query_as::<_, WorkflowRow>(
            "UPDATE workflows SET deleted_at = $1, updated_at = $2, updated_by = $3 WHERE id = $4 AND vault_id = $5 RETURNING *",
        )
        .bind(None::<DateTime<Utc>>)
        .bind(now)
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id)
        .bind(vault.0)
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        let workflow = Workflow::try_from(row)?;
        insert_domain_event(&mut tx, &workflow_event(&workflow, "restored")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(workflow)
    }

    #[instrument(skip(self, run))]
    async fn create_run(&self, run: &WorkflowRun) -> Result<bool> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let owner: Option<Id> = sqlx::query_scalar(
            "SELECT w.vault_id FROM workflows w JOIN vaults v ON v.id = w.vault_id
             WHERE w.id = $1 AND w.deleted_at IS NULL AND v.deleted_at IS NULL FOR SHARE OF w, v",
        )
        .bind(run.workflow_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?;
        if owner != Some(run.vault_id.0) {
            return Err(Error::Validation(
                "Workflow run belongs to an inactive workflow or vault, or a different vault"
                    .into(),
            ));
        }
        let mutation_id = run.source_identity()?.map(|source| source.mutation_id);
        let result = sqlx::query(
            "INSERT INTO workflow_runs (id, workflow_id, event, document_id, status, attempts,
                                        completed_actions, last_error, response_status, payload,
                                        actions, source_event_id, created_at, finished_at, mutation_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT DO NOTHING",
        )
        .bind(run.id)
        .bind(run.workflow_id)
        .bind(run.event.as_str())
        .bind(run.document_id)
        .bind(run.status.as_str())
        .bind(run.attempts as i32)
        .bind(run.completed_actions as i32)
        .bind(&run.last_error)
        .bind(run.response_status.map(i32::from))
        .bind(&run.payload)
        .bind(serde_json::to_string(&run.actions).map_err(repo_err)?)
        .bind(run.source_event_id)
        .bind(run.created_at)
        .bind(run.finished_at)
        .bind(mutation_id)
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        if result.rows_affected() == 0 {
            return Ok(false);
        }
        insert_job(
            &mut tx,
            JobType::ExecuteWorkflowRun { run_id: run.id },
            Utc::now(),
            MAX_ATTEMPTS as i32,
            crate::queue::JobTraceContext::current().as_ref(),
        )
        .await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(true)
    }

    #[instrument(skip(self))]
    async fn find_run(&self, id: Id) -> Result<Option<WorkflowRun>> {
        let row: Option<RunRow> = sqlx::query_as(
            "SELECT r.*, w.vault_id FROM workflow_runs r
             JOIN workflows w ON w.id = r.workflow_id WHERE r.id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.map(WorkflowRun::try_from).transpose()
    }

    #[instrument(skip(self, run))]
    async fn update_run(&self, run: &WorkflowRun) -> Result<()> {
        let updated = sqlx::query(
            "UPDATE workflow_runs SET status = $1, attempts = $2, completed_actions = $3,
                                      last_error = $4, response_status = $5, finished_at = $6
             WHERE id = $7 AND status = 'pending'",
        )
        .bind(run.status.as_str())
        .bind(run.attempts as i32)
        .bind(run.completed_actions as i32)
        .bind(&run.last_error)
        .bind(run.response_status.map(i32::from))
        .bind(run.finished_at)
        .bind(run.id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        if updated.rows_affected() == 0 {
            return Err(Error::Conflict(
                "Workflow run is already terminal or missing".into(),
            ));
        }
        Ok(())
    }

    #[instrument(skip(self, limit), fields(limit = i64::from(limit)))]
    async fn list_runs(
        &self,
        vault: VaultId,
        workflow_id: Id,
        limit: u32,
    ) -> Result<Vec<WorkflowRun>> {
        let rows: Vec<RunRow> = sqlx::query_as(
            "SELECT r.*, w.vault_id FROM workflow_runs r JOIN workflows w ON w.id = r.workflow_id
             WHERE r.workflow_id = $1 AND w.vault_id = $3 ORDER BY r.created_at DESC LIMIT $2",
        )
        .bind(workflow_id)
        .bind(i64::from(limit))
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(WorkflowRun::try_from).collect()
    }
}
