//! Workflow adapters: webhook delivery, run execution, and queue glue

mod runner;
mod webhook;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;

use folio_core::error::Result;
use folio_core::models::Id;
use folio_core::ports::{DocumentStore, FileMetadata, WorkflowJobQueue};

use crate::queue::{JobQueue, JobType};

pub use runner::{RunOutcome, WorkflowRunner, MAX_ATTEMPTS};
pub use webhook::{sign, DeliveryResult, WebhookClient, WebhookRequest, TIMEOUT};

#[async_trait]
impl WorkflowJobQueue for JobQueue {
    async fn enqueue_run(&self, run_id: Id) -> Result<()> {
        self.enqueue_with_max_attempts(
            JobType::ExecuteWorkflowRun { run_id },
            Utc::now(),
            MAX_ATTEMPTS as i32,
        )
        .await?;
        Ok(())
    }
}

/// Reads file sizes from the document store
pub struct StorageFileMetadata {
    store: Arc<dyn DocumentStore>,
}

impl StorageFileMetadata {
    pub fn new(store: Arc<dyn DocumentStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl FileMetadata for StorageFileMetadata {
    async fn size(&self, file_path: &str) -> Option<u64> {
        self.store.size(file_path).await.ok().flatten()
    }
}
