//! SQLite workflow repository implementation

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{sqlite::SqliteRow, Row, SqliteConnection, SqlitePool};
use tracing::instrument;

use super::insert_domain_event;
use crate::database::workflow_lifecycle::workflow_event;
use folio_core::error::{Error, Result};
use folio_core::events::EventOrigin;
use folio_core::models::{
    Id, RunStatus, UserId, VaultId, Workflow, WorkflowInput, WorkflowRun, WorkflowTrigger,
};
use folio_core::ports::WorkflowRepository;

use crate::queue::{insert_job_sqlite as insert_job, JobType};
use crate::workflow::MAX_ATTEMPTS;

/// SQLite-backed workflow repository
#[derive(Clone)]
pub struct SqliteWorkflowRepository {
    pool: SqlitePool,
}

impl SqliteWorkflowRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn repo_err(e: impl std::fmt::Display) -> Error {
    Error::Repository(e.to_string())
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(repo_err)
}

fn parse_id(value: &str) -> Result<Id> {
    Id::parse_str(value).map_err(repo_err)
}

fn parse_trigger(value: &str) -> Result<WorkflowTrigger> {
    WorkflowTrigger::parse(value).ok_or_else(|| repo_err(format!("Unknown trigger '{value}'")))
}

fn workflow_from_row(row: &SqliteRow) -> Result<Workflow> {
    Ok(Workflow {
        id: parse_id(row.get("id"))?,
        vault_id: VaultId(parse_id(row.get("vault_id"))?),
        name: row.get("name"),
        enabled: row.get::<i64, _>("enabled") != 0,
        trigger: parse_trigger(row.get("trigger_event"))?,
        conditions: serde_json::from_str(row.get("conditions")).map_err(repo_err)?,
        actions: serde_json::from_str(row.get("actions")).map_err(repo_err)?,
        created_at: parse_time(row.get("created_at"))?,
        updated_at: parse_time(row.get("updated_at"))?,
        revision: row.get("revision"),
        deleted_at: row
            .get::<Option<&str>, _>("deleted_at")
            .map(parse_time)
            .transpose()?,
        created_by: row.get::<Option<String>, _>("created_by").map(UserId),
        updated_by: row.get::<Option<String>, _>("updated_by").map(UserId),
    })
}

fn run_from_row(row: &SqliteRow) -> Result<WorkflowRun> {
    let document_id: Option<&str> = row.get("document_id");
    let finished_at: Option<&str> = row.get("finished_at");
    Ok(WorkflowRun {
        id: parse_id(row.get("id"))?,
        vault_id: VaultId(parse_id(row.get("vault_id"))?),
        workflow_id: parse_id(row.get("workflow_id"))?,
        event: parse_trigger(row.get("event"))?,
        document_id: document_id.map(parse_id).transpose()?,
        status: RunStatus::parse(row.get("status")),
        attempts: row.get::<i64, _>("attempts") as u32,
        completed_actions: row.get::<i64, _>("completed_actions") as u32,
        last_error: row.get("last_error"),
        response_status: row
            .get::<Option<i64>, _>("response_status")
            .map(|s| s as u16),
        payload: row.get("payload"),
        actions: serde_json::from_str(row.get("actions")).map_err(repo_err)?,
        source_event_id: row.get("source_event_id"),
        definition_revision: row.get("definition_revision"),
        created_at: parse_time(row.get("created_at"))?,
        finished_at: finished_at.map(parse_time).transpose()?,
    })
}

#[async_trait]
impl WorkflowRepository for SqliteWorkflowRepository {
    async fn locate(&self, id: Id) -> Result<Option<VaultId>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT vault_id FROM workflows WHERE id = ? AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM vaults v WHERE v.id = workflows.vault_id AND v.deleted_at IS NULL)")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await
                .map_err(repo_err)?;
        row.map(|(v,)| parse_id(&v).map(VaultId)).transpose()
    }

    #[instrument(skip(self))]
    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Workflow>> {
        let row = sqlx::query(
            "SELECT * FROM workflows WHERE id = ? AND vault_id = ? AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM vaults v WHERE v.id = workflows.vault_id AND v.deleted_at IS NULL)",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(workflow_from_row).transpose()
    }

    #[instrument(skip(self))]
    async fn list(&self, vault: VaultId) -> Result<Vec<Workflow>> {
        let rows = sqlx::query(
            "SELECT * FROM workflows WHERE vault_id = ? AND deleted_at IS NULL
             ORDER BY created_at, name",
        )
        .bind(vault.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(workflow_from_row).collect()
    }

    #[instrument(skip(self))]
    async fn list_enabled(
        &self,
        vault: VaultId,
        trigger: WorkflowTrigger,
    ) -> Result<Vec<Workflow>> {
        let rows = sqlx::query(
            "SELECT * FROM workflows WHERE vault_id = ? AND enabled = 1 AND deleted_at IS NULL AND EXISTS (SELECT 1 FROM vaults v WHERE v.id = workflows.vault_id AND v.deleted_at IS NULL) AND trigger_event = ?
             ORDER BY created_at",
        )
        .bind(vault.to_string())
        .bind(trigger.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(workflow_from_row).collect()
    }

    async fn list_deleted(&self, vault: VaultId) -> Result<Vec<Workflow>> {
        let rows = sqlx::query(
            "SELECT * FROM workflows WHERE vault_id = ? AND deleted_at IS NOT NULL ORDER BY deleted_at DESC, name",
        )
        .bind(vault.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(workflow_from_row).collect()
    }

    async fn find_deleted(&self, vault: VaultId, id: Id) -> Result<Option<Workflow>> {
        let row = sqlx::query(
            "SELECT * FROM workflows WHERE id = ? AND vault_id = ? AND deleted_at IS NOT NULL",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(workflow_from_row).transpose()
    }

    #[instrument(skip(self, input))]
    async fn create(&self, vault: VaultId, input: WorkflowInput) -> Result<Workflow> {
        input.validate()?;
        let id = Id::new_v4();
        let now = Utc::now().to_rfc3339();
        let actor = folio_core::mutation::current_actor();
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        let row = sqlx::query(
            "INSERT INTO workflows (id, vault_id, name, enabled, trigger_event, conditions, actions, created_at, updated_at, created_by, updated_by) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.trigger.as_str())
        .bind(serde_json::to_string(&input.conditions).map_err(repo_err)?)
        .bind(serde_json::to_string(&input.actions).map_err(repo_err)?)
        .bind(&now)
        .bind(&now)
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(actor.as_ref().map(UserId::as_str))
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        let workflow = workflow_from_row(&row)?;
        insert_domain_event(&mut tx, &workflow_event(&workflow, "created")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(workflow)
    }

    #[instrument(skip(self, input))]
    async fn replace(
        &self,
        vault: VaultId,
        id: Id,
        input: WorkflowInput,
        expected_revision: Option<i64>,
    ) -> Result<Workflow> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        let row = sqlx::query(
            "SELECT * FROM workflows WHERE id = ? AND vault_id = ? AND deleted_at IS NULL",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or_else(|| not_found(id))?;
        let existing = workflow_from_row(&row)?;
        existing.check_revision(expected_revision)?;
        input.validate()?;
        if existing.name == input.name
            && existing.enabled == input.enabled
            && existing.trigger == input.trigger
            && existing.conditions == input.conditions
            && existing.actions == input.actions
        {
            return Ok(existing);
        }
        let actor = folio_core::mutation::current_actor();
        let row = sqlx::query(
            "UPDATE workflows SET revision = revision + 1, name = ?, enabled = ?, trigger_event = ?, conditions = ?, actions = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? RETURNING *",
        )
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.trigger.as_str())
        .bind(serde_json::to_string(&input.conditions).map_err(repo_err)?)
        .bind(serde_json::to_string(&input.actions).map_err(repo_err)?)
        .bind(Utc::now().to_rfc3339())
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        let workflow = workflow_from_row(&row)?;
        let cancelled = sqlx::query_as(
            "UPDATE workflow_runs SET status = 'failed', last_error = 'Workflow definition changed',
             finished_at = ? WHERE workflow_id = ? AND status = 'pending' RETURNING id, workflow_id, payload",
        )
        .bind(workflow.updated_at.to_rfc3339())
        .bind(id.to_string())
        .fetch_all(&mut *tx)
        .await
        .map_err(repo_err)?;
        publish_cancelled_runs(&mut tx, vault, cancelled).await?;
        for event in crate::database::workflow_lifecycle::workflow_update_events(
            &workflow,
            existing.enabled,
        )? {
            insert_domain_event(&mut tx, &event).await?;
        }
        tx.commit().await.map_err(repo_err)?;
        Ok(workflow)
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        let row = sqlx::query("SELECT * FROM workflows WHERE id = ? AND vault_id = ?")
            .bind(id.to_string())
            .bind(vault.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(repo_err)?
            .ok_or_else(|| not_found(id))?;
        let existing = workflow_from_row(&row)?;
        if existing.deleted_at.is_some() {
            return Ok(());
        }
        let actor = folio_core::mutation::current_actor();
        let now = Utc::now().to_rfc3339();
        let row = sqlx::query(
            "UPDATE workflows SET revision = revision + 1, deleted_at = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? RETURNING *",
        )
        .bind(Some(&now))
        .bind(&now)
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        let workflow = workflow_from_row(&row)?;
        let cancelled = sqlx::query_as(
            "UPDATE workflow_runs SET status = 'failed', last_error = 'Workflow was deleted', finished_at = ? WHERE workflow_id = ? AND status = 'pending' RETURNING id, workflow_id, payload",
        )
        .bind(&now)
        .bind(id.to_string())
        .fetch_all(&mut *tx)
        .await
        .map_err(repo_err)?;
        publish_cancelled_runs(&mut tx, vault, cancelled).await?;
        insert_domain_event(&mut tx, &workflow_event(&workflow, "deleted")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }

    async fn restore(&self, vault: VaultId, id: Id) -> Result<Workflow> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        let row = sqlx::query("SELECT * FROM workflows WHERE id = ? AND vault_id = ?")
            .bind(id.to_string())
            .bind(vault.to_string())
            .fetch_optional(&mut *tx)
            .await
            .map_err(repo_err)?
            .ok_or_else(|| not_found(id))?;
        let existing = workflow_from_row(&row)?;
        if existing.deleted_at.is_none() {
            return Ok(existing);
        }
        let actor = folio_core::mutation::current_actor();
        let now = Utc::now().to_rfc3339();
        let row = sqlx::query(
            "UPDATE workflows SET revision = revision + 1, deleted_at = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? RETURNING *",
        )
        .bind(None::<String>)
        .bind(&now)
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        let workflow = workflow_from_row(&row)?;
        insert_domain_event(&mut tx, &workflow_event(&workflow, "restored")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(workflow)
    }

    #[instrument(skip(self, run))]
    async fn create_run(&self, run: &WorkflowRun) -> Result<bool> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        let definition: Option<(String, i64, i64)> = sqlx::query_as(
            "SELECT w.vault_id, w.revision, w.enabled FROM workflows w JOIN vaults v ON v.id = w.vault_id
             WHERE w.id = ? AND w.deleted_at IS NULL AND v.deleted_at IS NULL",
        )
        .bind(run.workflow_id.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?;
        let Some((_, revision, enabled)) =
            definition.filter(|(owner, _, _)| *owner == run.vault_id.to_string())
        else {
            return Err(Error::Validation(
                "Workflow run belongs to an inactive workflow or vault, or a different vault"
                    .into(),
            ));
        };
        if run
            .definition_revision
            .is_some_and(|captured| enabled == 0 || revision != captured)
        {
            return Ok(false);
        }
        let mutation_id = run.source_identity()?.map(|source| source.mutation_id);
        let result = sqlx::query(
            "INSERT INTO workflow_runs (id, workflow_id, event, document_id, status, attempts, completed_actions, \
             last_error, response_status, payload, actions, source_event_id, created_at, finished_at, mutation_id, definition_revision) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        )
        .bind(run.id.to_string())
        .bind(run.workflow_id.to_string())
        .bind(run.event.as_str())
        .bind(run.document_id.map(|d| d.to_string()))
        .bind(run.status.as_str())
        .bind(run.attempts)
        .bind(run.completed_actions)
        .bind(&run.last_error)
        .bind(run.response_status)
        .bind(&run.payload)
        .bind(serde_json::to_string(&run.actions).map_err(repo_err)?)
        .bind(run.source_event_id)
        .bind(run.created_at.to_rfc3339())
        .bind(run.finished_at.map(|t| t.to_rfc3339()))
        .bind(mutation_id.map(|id| id.to_string()))
        .bind(revision)
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
        let row = sqlx::query(
            "SELECT r.*, w.vault_id FROM workflow_runs r
             JOIN workflows w ON w.id = r.workflow_id WHERE r.id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(run_from_row).transpose()
    }

    #[instrument(skip(self, run))]
    async fn update_run(&self, run: &WorkflowRun) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        let updated = sqlx::query(
            "UPDATE workflow_runs SET status = ?, attempts = ?, completed_actions = ?, last_error = ?, \
             response_status = ?, finished_at = ? WHERE id = ? AND status = 'pending'",
        )
        .bind(run.status.as_str())
        .bind(run.attempts)
        .bind(run.completed_actions)
        .bind(&run.last_error)
        .bind(run.response_status)
        .bind(run.finished_at.map(|t| t.to_rfc3339()))
        .bind(run.id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        if updated.rows_affected() == 0 {
            return Err(Error::Conflict(
                "Workflow run is already terminal or missing".into(),
            ));
        }
        if run.status.is_terminal() {
            let (vault_id, workflow_id, payload): (String, String, String) = sqlx::query_as(
                "SELECT w.vault_id, r.workflow_id, r.payload FROM workflow_runs r
                 JOIN workflows w ON w.id = r.workflow_id WHERE r.id = ?",
            )
            .bind(run.id.to_string())
            .fetch_one(&mut *tx)
            .await
            .map_err(repo_err)?;
            let event = crate::database::workflow_lifecycle::workflow_run_outcome(
                VaultId(parse_id(&vault_id)?),
                run.id,
                parse_id(&workflow_id)?,
                run.status,
                &payload,
            )?;
            super::insert_domain_event_with_origin(&mut tx, &event, EventOrigin::Workflow).await?;
        }
        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }

    #[instrument(skip(self, limit), fields(limit = i64::from(limit)))]
    async fn list_runs(
        &self,
        vault: VaultId,
        workflow_id: Id,
        limit: u32,
    ) -> Result<Vec<WorkflowRun>> {
        let rows = sqlx::query(
            "SELECT r.*, w.vault_id FROM workflow_runs r JOIN workflows w ON w.id = r.workflow_id
             WHERE r.workflow_id = ? AND w.vault_id = ? ORDER BY r.created_at DESC LIMIT ?",
        )
        .bind(workflow_id.to_string())
        .bind(vault.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(run_from_row).collect()
    }
}

fn not_found(id: Id) -> Error {
    Error::NotFound {
        entity_type: "Workflow",
        id: id.to_string(),
    }
}

pub(super) async fn publish_cancelled_runs(
    conn: &mut SqliteConnection,
    vault: VaultId,
    runs: Vec<(String, String, String)>,
) -> Result<()> {
    for (id, workflow_id, payload) in runs {
        let event = crate::database::workflow_lifecycle::workflow_run_outcome(
            vault,
            parse_id(&id)?,
            parse_id(&workflow_id)?,
            RunStatus::Failed,
            &payload,
        )?;
        super::insert_domain_event_with_origin(conn, &event, EventOrigin::Workflow).await?;
    }
    Ok(())
}
