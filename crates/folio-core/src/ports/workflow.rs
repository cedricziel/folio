//! Ports used by the workflow engine

use async_trait::async_trait;

use crate::error::Result;
use crate::models::{Id, VaultId, Workflow, WorkflowInput, WorkflowRun, WorkflowTrigger};

/// Persistence for workflows and their runs
///
/// Workflows belong to a vault. Runs are addressed by their own id, which only
/// the engine and worker hold.
#[async_trait]
pub trait WorkflowRepository: Send + Sync {
    /// The vault a workflow belongs to, for the worker that executes its runs
    async fn locate(&self, id: Id) -> Result<Option<VaultId>>;

    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Workflow>>;

    async fn list(&self, vault: VaultId) -> Result<Vec<Workflow>>;

    /// Enabled workflows listening for `trigger`
    async fn list_enabled(&self, vault: VaultId, trigger: WorkflowTrigger)
        -> Result<Vec<Workflow>>;

    async fn create(&self, vault: VaultId, input: WorkflowInput) -> Result<Workflow>;

    /// Replace all fields of a workflow
    async fn update(&self, vault: VaultId, id: Id, input: WorkflowInput) -> Result<Workflow>;

    /// Delete a workflow and its runs
    async fn delete(&self, vault: VaultId, id: Id) -> Result<()>;

    /// Insert a run and queue its execution job in one transaction
    ///
    /// Returns false, queueing nothing, when the run already exists: either
    /// the same outbox event was handled before or a once-per-document run
    /// is already there.
    async fn create_run(&self, run: &WorkflowRun) -> Result<bool>;

    async fn find_run(&self, id: Id) -> Result<Option<WorkflowRun>>;

    /// Persist the mutable fields of a run
    async fn update_run(&self, run: &WorkflowRun) -> Result<()>;

    /// Newest runs first
    async fn list_runs(
        &self,
        vault: VaultId,
        workflow_id: Id,
        limit: u32,
    ) -> Result<Vec<WorkflowRun>>;
}

/// Facts about stored files that the document model does not carry
#[async_trait]
pub trait FileMetadata: Send + Sync {
    /// Size in bytes of the stored file, if it exists
    async fn size(&self, file_path: &str) -> Option<u64>;
}
