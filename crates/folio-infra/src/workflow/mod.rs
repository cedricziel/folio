//! Workflow adapters: webhook delivery, run execution, and queue glue

mod runner;
mod webhook;

use async_trait::async_trait;
use chrono::Utc;

use folio_core::error::Result;
use folio_core::models::Id;
use folio_core::ports::{FileMetadata, WorkflowJobQueue};

use crate::queue::{JobQueue, JobType};
use crate::storage::FileStorage;

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

/// Reads file sizes from the document storage directory
pub struct StorageFileMetadata {
    storage: FileStorage,
}

impl StorageFileMetadata {
    pub fn new(storage: FileStorage) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl FileMetadata for StorageFileMetadata {
    async fn size(&self, file_path: &str) -> Option<u64> {
        tokio::fs::metadata(self.storage.get_full_path(file_path))
            .await
            .ok()
            .map(|m| m.len())
    }
}
