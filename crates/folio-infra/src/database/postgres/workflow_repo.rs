//! PostgreSQL workflow repository implementation

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tracing::instrument;

use folio_core::error::{Error, Result};
use folio_core::models::{Id, RunStatus, Workflow, WorkflowInput, WorkflowRun, WorkflowTrigger};
use folio_core::ports::WorkflowRepository;

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
    name: String,
    enabled: bool,
    trigger_event: String,
    conditions: String,
    actions: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<WorkflowRow> for Workflow {
    type Error = Error;

    fn try_from(row: WorkflowRow) -> Result<Self> {
        Ok(Workflow {
            id: row.id,
            name: row.name,
            enabled: row.enabled,
            trigger: parse_trigger(&row.trigger_event)?,
            conditions: serde_json::from_str(&row.conditions).map_err(repo_err)?,
            actions: serde_json::from_str(&row.actions).map_err(repo_err)?,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct RunRow {
    id: Id,
    workflow_id: Id,
    event: String,
    document_id: Option<Id>,
    status: String,
    attempts: i32,
    completed_actions: i32,
    last_error: Option<String>,
    response_status: Option<i32>,
    payload: String,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
}

impl TryFrom<RunRow> for WorkflowRun {
    type Error = Error;

    fn try_from(row: RunRow) -> Result<Self> {
        Ok(WorkflowRun {
            id: row.id,
            workflow_id: row.workflow_id,
            event: parse_trigger(&row.event)?,
            document_id: row.document_id,
            status: RunStatus::parse(&row.status),
            attempts: row.attempts as u32,
            completed_actions: row.completed_actions as u32,
            last_error: row.last_error,
            response_status: row.response_status.map(|s| s as u16),
            payload: row.payload,
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
    #[instrument(skip(self))]
    async fn find_by_id(&self, id: Id) -> Result<Option<Workflow>> {
        let row: Option<WorkflowRow> = sqlx::query_as("SELECT * FROM workflows WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.map(Workflow::try_from).transpose()
    }

    #[instrument(skip(self))]
    async fn list(&self) -> Result<Vec<Workflow>> {
        let rows: Vec<WorkflowRow> =
            sqlx::query_as("SELECT * FROM workflows ORDER BY created_at, name")
                .fetch_all(&self.pool)
                .await
                .map_err(repo_err)?;
        rows.into_iter().map(Workflow::try_from).collect()
    }

    #[instrument(skip(self))]
    async fn list_enabled(&self, trigger: WorkflowTrigger) -> Result<Vec<Workflow>> {
        let rows: Vec<WorkflowRow> = sqlx::query_as(
            "SELECT * FROM workflows WHERE enabled AND trigger_event = $1 ORDER BY created_at",
        )
        .bind(trigger.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(Workflow::try_from).collect()
    }

    #[instrument(skip(self, input))]
    async fn create(&self, input: WorkflowInput) -> Result<Workflow> {
        input.validate()?;
        let now = Utc::now();
        let id = Id::new_v4();
        sqlx::query(
            "INSERT INTO workflows (id, name, enabled, trigger_event, conditions, actions, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
        )
        .bind(id)
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.trigger.as_str())
        .bind(serde_json::to_string(&input.conditions).map_err(repo_err)?)
        .bind(serde_json::to_string(&input.actions).map_err(repo_err)?)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;

        Ok(Workflow {
            id,
            name: input.name,
            enabled: input.enabled,
            trigger: input.trigger,
            conditions: input.conditions,
            actions: input.actions,
            created_at: now,
            updated_at: now,
        })
    }

    #[instrument(skip(self, input))]
    async fn update(&self, id: Id, input: WorkflowInput) -> Result<Workflow> {
        input.validate()?;
        let result = sqlx::query(
            "UPDATE workflows SET name = $1, enabled = $2, trigger_event = $3, conditions = $4,
                                  actions = $5, updated_at = $6
             WHERE id = $7",
        )
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.trigger.as_str())
        .bind(serde_json::to_string(&input.conditions).map_err(repo_err)?)
        .bind(serde_json::to_string(&input.actions).map_err(repo_err)?)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;

        if result.rows_affected() == 0 {
            return Err(not_found(id));
        }
        self.find_by_id(id).await?.ok_or_else(|| not_found(id))
    }

    #[instrument(skip(self))]
    async fn delete(&self, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM workflows WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        if result.rows_affected() == 0 {
            return Err(not_found(id));
        }
        Ok(())
    }

    #[instrument(skip(self, run))]
    async fn create_run(&self, run: &WorkflowRun) -> Result<bool> {
        let result = sqlx::query(
            "INSERT INTO workflow_runs (id, workflow_id, event, document_id, status, attempts,
                                        completed_actions, last_error, response_status, payload,
                                        created_at, finished_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
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
        .bind(run.created_at)
        .bind(run.finished_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() > 0)
    }

    #[instrument(skip(self))]
    async fn find_run(&self, id: Id) -> Result<Option<WorkflowRun>> {
        let row: Option<RunRow> = sqlx::query_as("SELECT * FROM workflow_runs WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.map(WorkflowRun::try_from).transpose()
    }

    #[instrument(skip(self, run))]
    async fn update_run(&self, run: &WorkflowRun) -> Result<()> {
        sqlx::query(
            "UPDATE workflow_runs SET status = $1, attempts = $2, completed_actions = $3,
                                      last_error = $4, response_status = $5, finished_at = $6
             WHERE id = $7",
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
        Ok(())
    }

    #[instrument(skip(self))]
    async fn list_runs(&self, workflow_id: Id, limit: u32) -> Result<Vec<WorkflowRun>> {
        let rows: Vec<RunRow> = sqlx::query_as(
            "SELECT * FROM workflow_runs WHERE workflow_id = $1 ORDER BY created_at DESC LIMIT $2",
        )
        .bind(workflow_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter().map(WorkflowRun::try_from).collect()
    }
}
