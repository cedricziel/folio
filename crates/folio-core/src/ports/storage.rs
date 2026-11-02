//! Document storage port
//!
//! Objects are addressed by opaque, slash-separated keys such as
//! `documents/{id}/original`. Keys are what the database stores; no adapter
//! detail (filesystem path, bucket, URL) leaks past this trait.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::models::Id;

/// Key layout for everything Folio stores for a document.
pub mod keys {
    use super::Id;

    /// Prefix shared by all objects of a document, with a trailing slash.
    pub fn document_prefix(id: Id) -> String {
        format!("documents/{id}/")
    }

    /// The uploaded file.
    pub fn original(id: Id) -> String {
        format!("documents/{id}/original")
    }

    /// The first-page thumbnail. Page 1 of the page viewer reuses it.
    pub fn thumbnail(id: Id) -> String {
        format!("documents/{id}/thumb.jpg")
    }

    /// The thumbnail for `page` (1-based); page 1 is [`thumbnail`].
    pub fn page(id: Id, page: u32) -> String {
        if page <= 1 {
            thumbnail(id)
        } else {
            format!("documents/{id}/pages/{page}.jpg")
        }
    }
}

/// An object being read: its size when known and a byte stream.
pub struct StoredObject {
    pub size: Option<u64>,
    pub stream: BoxStream<'static, Result<Bytes>>,
}

impl StoredObject {
    /// Collect the whole object in memory.
    pub async fn into_bytes(self) -> Result<Vec<u8>> {
        let chunks: Vec<Bytes> = self.stream.try_collect().await?;
        Ok(chunks.concat())
    }
}

/// A local copy of an object for tools that need a file path (`pdftoppm`).
/// The file is removed when the value is dropped.
pub struct LocalCopy {
    file: tempfile::TempPath,
}

impl LocalCopy {
    pub fn new(file: tempfile::TempPath) -> Self {
        Self { file }
    }

    pub fn path(&self) -> &Path {
        &self.file
    }
}

/// Storage for document files and derived artifacts
#[async_trait]
pub trait DocumentStore: Send + Sync {
    /// Short backend name for logs and metrics (`local`, `s3`, `memory`).
    fn backend(&self) -> &'static str;

    /// Write an object, replacing any existing one.
    async fn put(&self, key: &str, data: Bytes) -> Result<()>;

    /// Open an object for streaming. Missing keys yield [`Error::NotFound`].
    async fn get(&self, key: &str) -> Result<StoredObject>;

    /// Size in bytes of an object, or `None` when it does not exist.
    async fn size(&self, key: &str) -> Result<Option<u64>>;

    /// Whether an object exists.
    async fn exists(&self, key: &str) -> Result<bool>;

    /// Delete an object. Deleting a missing key succeeds.
    async fn delete(&self, key: &str) -> Result<()>;

    /// Delete every object under `prefix`, a key path ending in `/`; returns how many.
    async fn delete_prefix(&self, prefix: &str) -> Result<u64>;

    /// List the keys under `prefix`, a key path ending in `/`.
    async fn list(&self, prefix: &str) -> Result<Vec<String>>;

    /// A time-limited URL the client can fetch directly, if the backend
    /// supports it. Backends without support return `None` and callers
    /// stream through [`DocumentStore::get`].
    async fn presigned_url(&self, _key: &str, _expires_in: Duration) -> Result<Option<String>> {
        Ok(None)
    }

    /// Read a whole object into memory.
    async fn get_bytes(&self, key: &str) -> Result<Vec<u8>> {
        self.get(key).await?.into_bytes().await
    }

    /// Copy an object to a temporary local file.
    async fn download_to_temp(&self, key: &str) -> Result<LocalCopy> {
        let object = self.get(key).await?;
        let (file, path) = tempfile::NamedTempFile::new()
            .map_err(|e| Error::Storage(format!("Failed to create temp file: {e}")))?
            .into_parts();
        let mut file = tokio::fs::File::from_std(file);
        let mut stream = object.stream;
        while let Some(chunk) = stream.try_next().await? {
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                .await
                .map_err(|e| Error::Storage(format!("Failed to write temp file: {e}")))?;
        }
        tokio::io::AsyncWriteExt::flush(&mut file)
            .await
            .map_err(|e| Error::Storage(format!("Failed to write temp file: {e}")))?;
        Ok(LocalCopy::new(path))
    }
}

/// Remove every object under the document's storage prefix.
pub async fn purge_document(store: &dyn DocumentStore, id: Id) -> Result<()> {
    store.delete_prefix(&keys::document_prefix(id)).await?;
    Ok(())
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
