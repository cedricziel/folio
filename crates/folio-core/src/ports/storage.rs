//! File storage and background processing ports

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::models::Id;

/// Storage for original document files
#[async_trait]
pub trait FileStore: Send + Sync {
    /// Store a file and return its path relative to the storage root
    async fn store(&self, document_id: Id, filename: &str, data: &[u8]) -> Result<String>;

    /// Read a previously stored file by its relative path
    async fn read(&self, relative_path: &str) -> Result<Vec<u8>>;
}

/// Where a document is in the background processing pipeline
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingStatus {
    /// Queued, not started yet
    Pending,
    /// Currently being processed
    Processing,
    /// No outstanding or failed work
    Completed,
    /// At least one processing step failed
    Failed,
}

/// Background processing (thumbnail, OCR, embedding) of documents
#[async_trait]
pub trait ProcessingQueue: Send + Sync {
    /// Schedule full processing for a newly stored document
    async fn enqueue_processing(&self, document_id: Id) -> Result<()>;

    /// Current processing status of a document
    async fn processing_status(&self, document_id: Id) -> Result<ProcessingStatus>;
}
