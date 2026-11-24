//! The one path that turns bytes into a stored, queued document

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;

use super::NewUpload;
use crate::error::{Error, Result};
use crate::models::{CreateDocument, Document, FileIdentity, Id, LedgerEntry, VaultId};
use crate::ports::{keys, DocumentImporter, DocumentRepository, DocumentStore, ProcessingQueue};

/// Creates the record, stores the file, and queues background processing
///
/// Hand it a repository that announces events, so the new document triggers
/// `document.uploaded` workflows. Uploads, MCP, and ingestion sources all end
/// up here.
pub struct Importer {
    documents: Arc<dyn DocumentRepository>,
    files: Arc<dyn DocumentStore>,
    queue: Arc<dyn ProcessingQueue>,
}

impl Importer {
    pub fn new(
        documents: Arc<dyn DocumentRepository>,
        files: Arc<dyn DocumentStore>,
        queue: Arc<dyn ProcessingQueue>,
    ) -> Self {
        Self {
            documents,
            files,
            queue,
        }
    }
}

impl Importer {
    async fn store_original(&self, document: &Document, data: Vec<u8>) -> Result<FileIdentity> {
        let key = keys::original(document.id);
        let mime_type = document
            .mime_type
            .as_deref()
            .filter(|mime_type| !mime_type.is_empty())
            .unwrap_or("application/octet-stream");
        let data = Bytes::from(data);
        let expected = FileIdentity::from_bytes(&key, mime_type, &data);
        if !self.files.put_if_absent(&key, data).await? {
            let stored = self
                .files
                .get(&key)
                .await?
                .file_identity(&key, mime_type)
                .await?;
            if stored != expected {
                return Err(Error::Conflict(
                    "Stored original differs from this upload".into(),
                ));
            }
        }
        Ok(expected)
    }

    async fn publish(&self, vault: VaultId, document: &Document, data: Vec<u8>) -> Result<()> {
        let imported = async {
            let identity = self.store_original(document, data).await?;
            self.documents
                .finalize_verified_upload(vault, document.id, &identity)
                .await?;
            if let Err(e) = self.queue.enqueue_processing(vault, document.id).await {
                tracing::error!("Failed to queue processing of {}: {e}", document.id);
            }
            Ok(())
        }
        .await;
        if imported.is_err()
            && matches!(
                self.documents.is_purging_or_missing(document.id).await,
                Ok(true)
            )
        {
            // Purge cleanup can run before an in-flight original write finishes.
            if let Err(e) = self
                .files
                .delete_prefix(&keys::document_prefix(document.id))
                .await
            {
                tracing::warn!(
                    "Failed to remove late import objects for {}: {e}",
                    document.id
                );
            }
        }
        imported
    }

    async fn finish(&self, vault: VaultId, document: Document, data: Vec<u8>) -> Result<Document> {
        self.publish(vault, &document, data).await?;
        self.documents
            .find_by_id(vault, document.id)
            .await?
            .ok_or_else(|| crate::Error::NotFound {
                entity_type: "Document",
                id: document.id.to_string(),
            })
    }
}

#[async_trait]
impl DocumentImporter for Importer {
    async fn begin_as(&self, vault: VaultId, id: Id, upload: &NewUpload) -> Result<Document> {
        self.documents
            .create_for_upload_as(
                vault,
                id,
                CreateDocument {
                    uploaded_by: upload.uploaded_by.clone(),
                    title: upload
                        .title
                        .clone()
                        .unwrap_or_else(|| upload.filename.clone()),
                    original_filename: Some(upload.filename.clone()),
                    mime_type: upload.mime_type.clone(),
                    source: upload.source.clone(),
                    tag_ids: upload.tag_ids.clone(),
                    ..Default::default()
                },
            )
            .await
    }

    async fn import_as(&self, vault: VaultId, id: Id, upload: NewUpload) -> Result<()> {
        let document = self.begin_as(vault, id, &upload).await?;
        if document.file_path.is_some() {
            return Ok(());
        }
        self.publish(vault, &document, upload.data).await
    }

    async fn import_reserved(
        &self,
        vault: VaultId,
        claim: &LedgerEntry,
        upload: NewUpload,
    ) -> Result<Document> {
        let document = self
            .documents
            .create_for_ingest(
                vault,
                claim,
                CreateDocument {
                    uploaded_by: upload.uploaded_by,
                    title: upload.title.unwrap_or_else(|| upload.filename.clone()),
                    original_filename: Some(upload.filename),
                    mime_type: upload.mime_type,
                    source: upload.source,
                    tag_ids: upload.tag_ids,
                    ..Default::default()
                },
            )
            .await?;
        self.finish(vault, document, upload.data).await
    }

    async fn import(&self, vault: VaultId, upload: NewUpload) -> Result<Document> {
        let document = self
            .documents
            .create_for_upload(
                vault,
                CreateDocument {
                    uploaded_by: upload.uploaded_by,
                    title: upload.title.unwrap_or_else(|| upload.filename.clone()),
                    original_filename: Some(upload.filename.clone()),
                    mime_type: upload.mime_type,
                    source: upload.source,
                    tag_ids: upload.tag_ids,
                    ..Default::default()
                },
            )
            .await?;

        let stored = match self.store_original(&document, upload.data).await {
            Ok(identity) => {
                self.documents
                    .finalize_verified_upload(vault, document.id, &identity)
                    .await
            }
            Err(e) => Err(e),
        };
        if let Err(e) = stored {
            // Failed creation can leave an object behind even when its path was not saved.
            let cleanup = async {
                self.documents.delete(vault, document.id).await?;
                self.documents.request_purge(vault, document.id).await
            }
            .await;
            if let Err(cleanup_error) = cleanup {
                tracing::error!(
                    "Failed to schedule cleanup of incomplete upload {}: {cleanup_error}",
                    document.id
                );
            }
            return Err(e);
        }
        // The file is stored, so a failed enqueue leaves a usable document
        // that can be reprocessed.
        if let Err(e) = self.queue.enqueue_processing(vault, document.id).await {
            tracing::error!("Failed to queue processing of {}: {e}", document.id);
        }

        Ok(self
            .documents
            .find_by_id(vault, document.id)
            .await?
            .unwrap_or(document))
    }
}
