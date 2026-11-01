//! The one path that turns bytes into a stored, queued document

use std::sync::Arc;

use async_trait::async_trait;

use super::NewUpload;
use crate::error::Result;
use crate::models::{CreateDocument, Document};
use crate::ports::{DocumentImporter, DocumentRepository, FileStore, ProcessingQueue};

/// Creates the record, stores the file, and queues background processing
///
/// Hand it a repository that announces events, so the new document triggers
/// `document.uploaded` workflows. Uploads, MCP, and ingestion sources all end
/// up here.
pub struct Importer {
    documents: Arc<dyn DocumentRepository>,
    files: Arc<dyn FileStore>,
    queue: Arc<dyn ProcessingQueue>,
}

impl Importer {
    pub fn new(
        documents: Arc<dyn DocumentRepository>,
        files: Arc<dyn FileStore>,
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

        match self
            .files
            .store(document.id, &upload.filename, &upload.data)
            .await
        {
            Ok(path) => self.documents.set_file_path(document.id, &path).await?,
            Err(e) => {
                // A record without its file is useless; do not leave one behind.
                let _ = self.documents.delete(document.id).await;
                return Err(e);
            }
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
