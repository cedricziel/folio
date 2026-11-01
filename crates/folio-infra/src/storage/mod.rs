//! Document storage adapters
//!
//! One adapter over the `object_store` crate serves every backend: the local
//! filesystem, S3-compatible services, and an in-memory store for tests.

mod legacy;
mod object_store_adapter;

use std::sync::Arc;

use folio_core::error::{Error, Result};
use folio_core::ports::DocumentStore;

use crate::config::{StorageBackend, StorageConfig};

pub use legacy::{cleanup_legacy_layout, migrate_legacy_layout};
pub use object_store_adapter::ObjectStoreDocumentStore;

/// Build the store selected by `storage.backend`.
pub async fn build_store(config: &StorageConfig) -> Result<Arc<dyn DocumentStore>> {
    match config.backend {
        StorageBackend::Local => Ok(Arc::new(
            ObjectStoreDocumentStore::local(&config.path).await?,
        )),
        StorageBackend::S3 => Ok(Arc::new(ObjectStoreDocumentStore::s3(&config.s3)?)),
    }
}

pub(crate) fn storage_err(context: &str, e: impl std::fmt::Display) -> Error {
    Error::Storage(format!("{context}: {e}"))
}
