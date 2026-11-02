//! Emits lifecycle events for every change made through the wrapped repository

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::Result;
use crate::events::{DeletedDocument, DocumentEvent, EventBroadcaster};
use crate::models::{
    CreateDocument, Document, DocumentFilter, Id, ProcessingStage, ProcessingStatus, Tag,
    UpdateDocument,
};
use crate::ports::DocumentRepository;

/// A [`DocumentRepository`] that announces creates, updates, and deletes
///
/// This is the single source of lifecycle events: SSE clients and the
/// workflow engine both consume what it broadcasts, so no HTTP handler has to
/// remember to emit anything. Bookkeeping writes (`set_file_path`,
/// `set_page_count`) stay silent. Background work that manages its own events,
/// like the job worker, should use the unwrapped repository.
pub struct EventingDocumentRepository {
    inner: Arc<dyn DocumentRepository>,
    events: EventBroadcaster,
}

impl EventingDocumentRepository {
    pub fn new(inner: Arc<dyn DocumentRepository>, events: EventBroadcaster) -> Self {
        Self { inner, events }
    }
}

#[async_trait]
impl DocumentRepository for EventingDocumentRepository {
    async fn find_by_id(&self, id: Id) -> Result<Option<Document>> {
        self.inner.find_by_id(id).await
    }

    async fn list(&self, filter: Option<DocumentFilter>) -> Result<Vec<Document>> {
        self.inner.list(filter).await
    }

    async fn list_ids(&self, filter: Option<DocumentFilter>) -> Result<Vec<Id>> {
        self.inner.list_ids(filter).await
    }

    async fn create(&self, params: CreateDocument) -> Result<Document> {
        let document = self.inner.create(params).await?;
        self.events.send(DocumentEvent::Created {
            document_id: document.id,
        });
        Ok(document)
    }

    async fn update(&self, id: Id, params: UpdateDocument) -> Result<Document> {
        let document = self.inner.update(id, params).await?;
        self.events.send(DocumentEvent::Updated { document_id: id });
        Ok(document)
    }

    async fn delete(&self, id: Id) -> Result<()> {
        let snapshot = match self.inner.find_by_id(id).await? {
            Some(document) => Some(Arc::new(DeletedDocument {
                tags: self.inner.get_tags(id).await?,
                document,
            })),
            None => None,
        };
        self.inner.delete(id).await?;
        self.events.send(DocumentEvent::Deleted {
            document_id: id,
            snapshot,
        });
        Ok(())
    }

    async fn set_file_path(&self, id: Id, file_path: &str) -> Result<()> {
        self.inner.set_file_path(id, file_path).await
    }

    async fn set_page_count(&self, id: Id, page_count: u32) -> Result<()> {
        self.inner.set_page_count(id, page_count).await
    }

    async fn set_stage_status(
        &self,
        id: Id,
        stage: ProcessingStage,
        status: ProcessingStatus,
    ) -> Result<()> {
        self.inner.set_stage_status(id, stage, status).await
    }

    async fn complete_ocr(&self, id: Id, content: &str) -> Result<()> {
        self.inner.complete_ocr(id, content).await
    }

    async fn list_pending(&self, stage: ProcessingStage) -> Result<Vec<Document>> {
        self.inner.list_pending(stage).await
    }

    async fn reset_embedding_status(&self) -> Result<Vec<Id>> {
        self.inner.reset_embedding_status().await
    }

    async fn get_tags(&self, document_id: Id) -> Result<Vec<Tag>> {
        self.inner.get_tags(document_id).await
    }

    async fn set_tags(&self, document_id: Id, tag_ids: &[Id]) -> Result<()> {
        self.inner.set_tags(document_id, tag_ids).await?;
        self.events.send(DocumentEvent::Updated { document_id });
        Ok(())
    }

    async fn add_tag(&self, document_id: Id, tag_id: Id) -> Result<()> {
        self.inner.add_tag(document_id, tag_id).await?;
        self.events.send(DocumentEvent::Updated { document_id });
        Ok(())
    }

    async fn remove_tag(&self, document_id: Id, tag_id: Id) -> Result<()> {
        self.inner.remove_tag(document_id, tag_id).await?;
        self.events.send(DocumentEvent::Updated { document_id });
        Ok(())
    }
}
