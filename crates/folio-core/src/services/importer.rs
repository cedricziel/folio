//! The one path that turns bytes into a stored, queued document

use std::sync::Arc;

use async_trait::async_trait;

use super::NewUpload;
use crate::error::Result;
use crate::models::{CreateDocument, Document};
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

#[async_trait]
impl DocumentImporter for Importer {
    async fn import(&self, upload: NewUpload) -> Result<Document> {
        let document = self
            .documents
            .create(CreateDocument {
                title: upload.title.unwrap_or_else(|| upload.filename.clone()),
                original_filename: Some(upload.filename.clone()),
                mime_type: upload.mime_type,
                source: upload.source,
                tag_ids: upload.tag_ids,
                ..Default::default()
            })
            .await?;

        let key = keys::original(document.id);
        let stored = match self.files.put(&key, upload.data.into()).await {
            Ok(()) => self.documents.set_file_path(document.id, &key).await,
            Err(e) => Err(e),
        };
        if let Err(e) = stored {
            // A record without its file is useless; do not leave one behind.
            let _ = self.documents.delete(document.id).await;
            return Err(e);
        }
        // The file is stored, so a failed enqueue leaves a usable document
        // that can be reprocessed.
        if let Err(e) = self.queue.enqueue_processing(document.id).await {
            tracing::error!("Failed to queue processing of {}: {e}", document.id);
        }

        Ok(self
            .documents
            .find_by_id(document.id)
            .await?
            .unwrap_or(document))
    }
}
