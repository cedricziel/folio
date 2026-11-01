//! The one path that turns bytes into a stored, queued document

use std::sync::Arc;

use async_trait::async_trait;
use tracing::error;

use folio_core::error::Result;
use folio_core::models::{CreateDocument, Document};
use folio_core::ports::{DocumentImporter, DocumentRepository, ImportRequest};

use crate::queue::{JobQueue, JobType};
use crate::storage::FileStorage;

/// Creates the record, stores the file, and queues background processing
///
/// Hand it a repository that announces events, so the new document triggers
/// `document.uploaded` workflows.
pub struct StorageDocumentImporter {
    documents: Arc<dyn DocumentRepository>,
    storage: FileStorage,
    jobs: JobQueue,
}

impl StorageDocumentImporter {
    pub fn new(
        documents: Arc<dyn DocumentRepository>,
        storage: FileStorage,
        jobs: JobQueue,
    ) -> Self {
        Self {
            documents,
            storage,
            jobs,
        }
    }
}

#[async_trait]
impl DocumentImporter for StorageDocumentImporter {
    async fn import(&self, request: ImportRequest) -> Result<Document> {
        let document = self
            .documents
            .create(CreateDocument {
                title: request.title,
                original_filename: Some(request.filename.clone()),
                mime_type: Some(request.mime_type),
                source: request.source,
                tag_ids: request.tag_ids,
                ..Default::default()
            })
            .await?;

        match self
            .storage
            .store(document.id, &request.filename, &request.data)
            .await
        {
            Ok(path) => self.documents.set_file_path(document.id, &path).await?,
            Err(e) => {
                // A record without its file is useless; do not leave one behind.
                let _ = self.documents.delete(document.id).await;
                return Err(e);
            }
        }

        if let Err(e) = self
            .jobs
            .enqueue(JobType::ProcessDocument {
                document_id: document.id,
            })
            .await
        {
            error!("Failed to enqueue processing of {}: {e}", document.id);
        }

        Ok(self
            .documents
            .find_by_id(document.id)
            .await?
            .unwrap_or(document))
    }
}
