//! Ports used by the workflow engine

use async_trait::async_trait;

use crate::error::Result;
use crate::models::{Id, Workflow, WorkflowInput, WorkflowRun, WorkflowTrigger};

/// Persistence for workflows and their runs
#[async_trait]
pub trait WorkflowRepository: Send + Sync {
    async fn find_by_id(&self, id: Id) -> Result<Option<Workflow>>;

    async fn list(&self) -> Result<Vec<Workflow>>;

    /// Enabled workflows listening for `trigger`
    async fn list_enabled(&self, trigger: WorkflowTrigger) -> Result<Vec<Workflow>>;

    async fn create(&self, input: WorkflowInput) -> Result<Workflow>;

    /// Replace all fields of a workflow
    async fn update(&self, id: Id, input: WorkflowInput) -> Result<Workflow>;

    /// Delete a workflow and its runs
    async fn delete(&self, id: Id) -> Result<()>;

    async fn create_run(&self, run: &WorkflowRun) -> Result<()>;

    async fn find_run(&self, id: Id) -> Result<Option<WorkflowRun>>;

    /// Persist the mutable fields of a run
    async fn update_run(&self, run: &WorkflowRun) -> Result<()>;

    /// Newest runs first
    async fn list_runs(&self, workflow_id: Id, limit: u32) -> Result<Vec<WorkflowRun>>;

    /// Whether the workflow already has a run for this event and document
    async fn run_exists(
        &self,
        workflow_id: Id,
        event: WorkflowTrigger,
        document_id: Id,
    ) -> Result<bool>;
}

/// Hands a run to the background worker
#[async_trait]
pub trait WorkflowJobQueue: Send + Sync {
    async fn enqueue_run(&self, run_id: Id) -> Result<()>;
}

/// Facts about stored files that the document model does not carry
#[async_trait]
pub trait FileMetadata: Send + Sync {
    /// Size in bytes of the stored file, if it exists
    async fn size(&self, file_path: &str) -> Option<u64>;
}
