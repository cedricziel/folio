//! Workflow adapters: webhook delivery and run execution

mod runner;
mod webhook;

use std::sync::Arc;

use async_trait::async_trait;

use folio_core::ports::{DocumentStore, FileMetadata};

pub use runner::{RunOutcome, WorkflowRunner, MAX_ATTEMPTS};
pub use webhook::{sign, DeliveryResult, WebhookClient, WebhookRequest, TIMEOUT};

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
