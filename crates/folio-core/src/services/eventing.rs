//! Emits lifecycle events for every change made through the wrapped repository

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::Result;
use crate::events::{DeletedDocument, DocumentEvent, EventBroadcaster};
use crate::models::{
    CreateDocument, Document, DocumentFilter, Id, ProcessingStage, ProcessingStatus, Tag,
    UpdateDocument, UserId, VaultId,
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
    async fn locate(&self, id: Id) -> Result<Option<VaultId>> {
        self.inner.locate(id).await
    }

    async fn move_to(
        &self,
        id: Id,
        from: VaultId,
        to: VaultId,
        actor: Option<&UserId>,
    ) -> Result<Document> {
        self.inner.move_to(id, from, to, actor).await
    }

    async fn find_by_id(&self, vault: VaultId, id: Id) -> Result<Option<Document>> {
        self.inner.find_by_id(vault, id).await
    }

    async fn list(&self, vault: VaultId, filter: Option<DocumentFilter>) -> Result<Vec<Document>> {
        self.inner.list(vault, filter).await
    }

    async fn create(&self, vault: VaultId, params: CreateDocument) -> Result<Document> {
        let document = self.inner.create(vault, params).await?;
        self.events.send(DocumentEvent::Created {
            document_id: document.id,
        });
        Ok(document)
    }

    async fn update(&self, vault: VaultId, id: Id, params: UpdateDocument) -> Result<Document> {
        let document = self.inner.update(vault, id, params).await?;
        self.events.send(DocumentEvent::Updated { document_id: id });
        Ok(document)
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let snapshot = match self.inner.find_by_id(vault, id).await? {
            Some(document) => Some(Arc::new(DeletedDocument {
                tags: self.inner.get_tags(vault, id).await?,
                document,
            })),
            None => None,
        };
        self.inner.delete(vault, id).await?;
        self.events.send(DocumentEvent::Deleted {
            document_id: id,
            snapshot,
        });
        Ok(())
    }

    async fn set_file_path(&self, vault: VaultId, id: Id, file_path: &str) -> Result<()> {
        self.inner.set_file_path(vault, id, file_path).await
    }

    async fn set_page_count(&self, vault: VaultId, id: Id, page_count: u32) -> Result<()> {
        self.inner.set_page_count(vault, id, page_count).await
    }

    async fn set_stage_status(
        &self,
        vault: VaultId,
        id: Id,
        stage: ProcessingStage,
        status: ProcessingStatus,
    ) -> Result<()> {
        self.inner.set_stage_status(vault, id, stage, status).await
    }

    async fn complete_ocr(&self, vault: VaultId, id: Id, content: &str) -> Result<()> {
        self.inner.complete_ocr(vault, id, content).await
    }

    async fn list_all(&self) -> Result<Vec<Document>> {
        self.inner.list_all().await
    }

    async fn list_pending(&self, stage: ProcessingStage) -> Result<Vec<Document>> {
        self.inner.list_pending(stage).await
    }

    async fn reset_embedding_status(&self) -> Result<Vec<Id>> {
        self.inner.reset_embedding_status().await
    }

    async fn get_tags(&self, vault: VaultId, document_id: Id) -> Result<Vec<Tag>> {
        self.inner.get_tags(vault, document_id).await
    }

    async fn set_tags(&self, vault: VaultId, document_id: Id, tag_ids: &[Id]) -> Result<()> {
        self.inner.set_tags(vault, document_id, tag_ids).await?;
        self.events.send(DocumentEvent::Updated { document_id });
        Ok(())
    }

    async fn add_tag(&self, vault: VaultId, document_id: Id, tag_id: Id) -> Result<()> {
        self.inner.add_tag(vault, document_id, tag_id).await?;
        self.events.send(DocumentEvent::Updated { document_id });
        Ok(())
    }

    async fn remove_tag(&self, vault: VaultId, document_id: Id, tag_id: Id) -> Result<()> {
        self.inner.remove_tag(vault, document_id, tag_id).await?;
        self.events.send(DocumentEvent::Updated { document_id });
        Ok(())
    }
}
