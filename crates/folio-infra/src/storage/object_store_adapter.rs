//! `DocumentStore` over any `object_store` backend

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, TryStreamExt};
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::prefix::PrefixStore;
use object_store::signer::Signer;
use object_store::{ObjectStore, ObjectStoreExt};

use folio_core::error::{Error, Result};
use folio_core::ports::{DocumentStore, StoredObject};

use super::storage_err;
use crate::config::S3Config;

pub struct ObjectStoreDocumentStore {
    store: Arc<dyn ObjectStore>,
    backend: &'static str,
    signer: Option<Arc<AmazonS3>>,
    /// Key prefix the signer must add, because it sees the unprefixed bucket.
    prefix: Option<Path>,
}

impl ObjectStoreDocumentStore {
    /// Files under `root`; the directory is created when missing.
    pub async fn local(root: &str) -> Result<Self> {
        tokio::fs::create_dir_all(root)
            .await
            .map_err(|e| storage_err("Failed to create storage directory", e))?;
        let store = LocalFileSystem::new_with_prefix(root)
            .map_err(|e| storage_err("Failed to open local storage", e))?
            .with_automatic_cleanup(true);
        Ok(Self {
            store: Arc::new(store),
            backend: "local",
            signer: None,
            prefix: None,
        })
    }

    /// An S3-compatible bucket. Credentials fall back to the standard `AWS_*`
    /// environment variables and instance metadata when not configured.
    pub fn s3(config: &S3Config) -> Result<Self> {
        let bucket = config
            .bucket
            .as_deref()
            .filter(|b| !b.is_empty())
            .ok_or_else(|| Error::Storage("storage.s3.bucket is required".into()))?;

        // Custom endpoints (MinIO, Garage, R2) are addressed by path unless told otherwise.
        let path_style = config.path_style.unwrap_or(config.endpoint.is_some());
        let mut builder = AmazonS3Builder::from_env()
            .with_bucket_name(bucket)
            .with_virtual_hosted_style_request(!path_style)
            .with_allow_http(config.allow_http);
        if let Some(region) = &config.region {
            builder = builder.with_region(region);
        }
        if let Some(endpoint) = &config.endpoint {
            builder = builder.with_endpoint(endpoint);
        }
        if let Some(key) = &config.access_key_id {
            builder = builder.with_access_key_id(key);
        }
        if let Some(secret) = &config.secret_access_key {
            builder = builder.with_secret_access_key(secret);
        }
        let s3 = Arc::new(
            builder
                .build()
                .map_err(|e| storage_err("Failed to configure S3 storage", e))?,
        );

        let prefix = config
            .prefix
            .as_deref()
            .map(|p| p.trim_matches('/'))
            .filter(|p| !p.is_empty())
            .map(Path::from);
        let store: Arc<dyn ObjectStore> = match &prefix {
            Some(prefix) => Arc::new(PrefixStore::new(s3.clone(), prefix.clone())),
            None => s3.clone(),
        };
        Ok(Self {
            store,
            backend: "s3",
            signer: Some(s3),
            prefix,
        })
    }

    /// Volatile store for tests.
    pub fn in_memory() -> Self {
        Self {
            store: Arc::new(InMemory::new()),
            backend: "memory",
            signer: None,
            prefix: None,
        }
    }

    fn path(key: &str) -> Result<Path> {
        Path::parse(key).map_err(|e| Error::Storage(format!("Invalid object key {key:?}: {e}")))
    }

    fn map_get_error(key: &str, e: object_store::Error) -> Error {
        match e {
            object_store::Error::NotFound { .. } => Error::NotFound {
                entity_type: "Object",
                id: key.to_string(),
            },
            other => storage_err(&format!("Failed to read {key}"), other),
        }
    }
}

#[async_trait]
impl DocumentStore for ObjectStoreDocumentStore {
    fn backend(&self) -> &'static str {
        self.backend
    }

    async fn put(&self, key: &str, data: Bytes) -> Result<()> {
        self.store
            .put(&Self::path(key)?, data.into())
            .await
            .map_err(|e| storage_err(&format!("Failed to write {key}"), e))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<StoredObject> {
        let result = self
            .store
            .get(&Self::path(key)?)
            .await
            .map_err(|e| Self::map_get_error(key, e))?;
        let size = Some(result.meta.size);
        let owned_key = key.to_string();
        let stream = result
            .into_stream()
            .map_err(move |e| storage_err(&format!("Failed to read {owned_key}"), e))
            .boxed();
        Ok(StoredObject { size, stream })
    }

    async fn size(&self, key: &str) -> Result<Option<u64>> {
        match self.store.head(&Self::path(key)?).await {
            Ok(meta) => Ok(Some(meta.size)),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(storage_err(&format!("Failed to stat {key}"), e)),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        match self.store.head(&Self::path(key)?).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(storage_err(&format!("Failed to stat {key}"), e)),
        }
    }

    async fn copy(&self, from: &str, to: &str) -> Result<()> {
        self.store
            .copy(&Self::path(from)?, &Self::path(to)?)
            .await
            .map_err(|e| Self::map_get_error(from, e))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        match self.store.delete(&Self::path(key)?).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(storage_err(&format!("Failed to delete {key}"), e)),
        }
    }

    async fn delete_prefix(&self, prefix: &str) -> Result<u64> {
        let keys = self.list(prefix).await?;
        let mut deleted = 0;
        for key in keys {
            self.delete(&key).await?;
            deleted += 1;
        }
        Ok(deleted)
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let prefix = Self::path(prefix.trim_end_matches('/'))?;
        self.store
            .list(Some(&prefix))
            .map_ok(|meta| meta.location.to_string())
            .try_collect()
            .await
            .map_err(|e| storage_err(&format!("Failed to list {prefix}"), e))
    }

    async fn presigned_url(&self, key: &str, expires_in: Duration) -> Result<Option<String>> {
        let Some(signer) = &self.signer else {
            return Ok(None);
        };
        let path = Self::path(key)?;
        let path = match &self.prefix {
            Some(prefix) => prefix.parts().chain(path.parts()).collect(),
            None => path,
        };
        let url = signer
            .signed_url(object_store::signer::Method::GET, &path, expires_in)
            .await
            .map_err(|e| storage_err("Failed to sign URL", e))?;
        Ok(Some(url.to_string()))
    }
}
