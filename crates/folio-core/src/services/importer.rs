//! The one path that turns bytes into a stored, queued document

use std::sync::Arc;

use async_trait::async_trait;

use super::NewUpload;
use crate::error::Result;
use crate::models::{CreateDocument, Document, Id, LedgerEntry, VaultId};
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
    /// Stores the original of a created upload unless it is already there, finalizes it and queues processing.
    /// Every step can be repeated, so an interrupted import is resumed by running it again.
    async fn finish(&self, vault: VaultId, document: Document, data: Vec<u8>) -> Result<Document> {
        let file_size = data.len() as u64;
        let imported = async {
            let key = keys::original(document.id);
            if !self.files.exists(&key).await? {
                self.files.put(&key, data.into()).await?;
            }
            self.documents
                .finalize_upload(vault, document.id, &key, file_size)
                .await?;
            if let Err(e) = self.queue.enqueue_processing(document.id).await {
                tracing::error!("Failed to queue processing of {}: {e}", document.id);
            }
            self.documents
                .find_by_id(vault, document.id)
                .await?
                .ok_or_else(|| crate::Error::NotFound {
                    entity_type: "Document",
                    id: document.id.to_string(),
                })
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

    async fn import_as(&self, vault: VaultId, id: Id, upload: NewUpload) -> Result<Document> {
        let document = self.begin_as(vault, id, &upload).await?;
        if document.file_path.is_some() {
            return Ok(document);
        }
        self.finish(vault, document, upload.data).await
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

        let key = keys::original(document.id);
        let file_size = upload.data.len() as u64;
        let stored = match self.files.put(&key, upload.data.into()).await {
            Ok(()) => {
                self.documents
                    .finalize_upload(vault, document.id, &key, file_size)
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
        if let Err(e) = self.queue.enqueue_processing(document.id).await {
            tracing::error!("Failed to queue processing of {}: {e}", document.id);
        }

        Ok(self
            .documents
            .find_by_id(vault, document.id)
            .await?
            .unwrap_or(document))
    }
}
