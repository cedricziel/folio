//! Business services
//!
//! This module contains the core business logic that orchestrates
//! domain operations using the port traits.

use std::sync::Arc;

use serde::Serialize;

use crate::error::Result;
use crate::models::{CreateDocument, Document, DocumentFilter, Id};
use crate::ports::{DocumentRepository, EmbeddingService, OcrService, SearchResult, SearchService};

/// Service for managing documents
pub struct DocumentService {
    document_repo: Arc<dyn DocumentRepository>,
}

impl DocumentService {
    /// Create a new document service
    pub fn new(document_repo: Arc<dyn DocumentRepository>) -> Self {
        Self { document_repo }
    }

    /// Get a document by ID
    pub async fn get(&self, id: Id) -> Result<Option<Document>> {
        self.document_repo.find_by_id(id).await
    }

    /// List documents with optional filtering
    pub async fn list(&self, filter: Option<DocumentFilter>) -> Result<Vec<Document>> {
        self.document_repo.list(filter).await
    }

    /// Create a new document
    pub async fn create(&self, params: CreateDocument) -> Result<Document> {
        self.document_repo.create(params).await
    }

    /// Delete a document
    pub async fn delete(&self, id: Id) -> Result<()> {
        self.document_repo.delete(id).await
    }
}

/// Service for ingesting new documents (OCR, embedding, storage)
pub struct IngestService {
    document_repo: Arc<dyn DocumentRepository>,
    ocr_service: Arc<dyn OcrService>,
    embedding_service: Arc<dyn EmbeddingService>,
    search_service: Arc<dyn SearchService>,
}

impl IngestService {
    /// Create a new ingest service
    pub fn new(
        document_repo: Arc<dyn DocumentRepository>,
        ocr_service: Arc<dyn OcrService>,
        embedding_service: Arc<dyn EmbeddingService>,
        search_service: Arc<dyn SearchService>,
    ) -> Self {
        Self {
            document_repo,
            ocr_service,
            embedding_service,
            search_service,
        }
    }

    /// Ingest a new document
    ///
    /// This performs the full ingestion pipeline:
    /// 1. Create the document record
    /// 2. Extract text via OCR (if needed)
    /// 3. Generate embedding
    /// 4. Store embedding for search
    pub async fn ingest(
        &self,
        title: String,
        data: &[u8],
        filename: Option<String>,
        mime_type: String,
    ) -> Result<Document> {
        // Extract text from document
        let content = self.ocr_service.extract_text(data, &mime_type).await?;

        // Create document record
        let doc = self
            .document_repo
            .create(CreateDocument {
                title,
                content: Some(content.clone()),
                original_filename: filename,
                mime_type: Some(mime_type),
                correspondent_id: None,
                document_type_id: None,
            })
            .await?;

        // Generate and store embedding for search
        if !content.is_empty() {
            let embedding = self.embedding_service.generate_embedding(&content).await?;
            self.search_service
                .store_embedding(doc.id, &embedding)
                .await?;
        }

        Ok(doc)
    }

    /// Re-process an existing document (regenerate OCR and embedding)
    pub async fn reprocess(&self, id: Id, data: &[u8], mime_type: &str) -> Result<Document> {
        // Get existing document
        let _existing = self.document_repo.find_by_id(id).await?.ok_or_else(|| {
            crate::error::Error::NotFound {
                entity_type: "Document",
                id: id.to_string(),
            }
        })?;

        // Extract text
        let content = self.ocr_service.extract_text(data, mime_type).await?;

        // Update document content
        let doc = self
            .document_repo
            .update(
                id,
                crate::models::UpdateDocument {
                    content: Some(content.clone()),
                    ..Default::default()
                },
            )
            .await?;

        // Regenerate embedding
        if !content.is_empty() {
            let embedding = self.embedding_service.generate_embedding(&content).await?;
            self.search_service.store_embedding(id, &embedding).await?;
        }

        Ok(doc)
    }
}

/// A document together with its relevance score for a search
#[derive(Debug, Clone, Serialize)]
pub struct ScoredDocument {
    pub document: Document,
    pub score: f32,
}

/// Service for semantic search
pub struct SearchServiceFacade {
    document_repo: Arc<dyn DocumentRepository>,
    embedding_service: Arc<dyn EmbeddingService>,
    search_service: Arc<dyn SearchService>,
}

impl SearchServiceFacade {
    /// Create a new search service facade
    pub fn new(
        document_repo: Arc<dyn DocumentRepository>,
        embedding_service: Arc<dyn EmbeddingService>,
        search_service: Arc<dyn SearchService>,
    ) -> Self {
        Self {
            document_repo,
            embedding_service,
            search_service,
        }
    }

    /// Search for documents using natural language query
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let query_embedding = self.embedding_service.generate_embedding(query).await?;
        self.search_service.search(&query_embedding, limit).await
    }

    /// Search by natural language query and resolve hits to full documents,
    /// skipping hits whose document no longer exists
    pub async fn search_documents(&self, query: &str, limit: usize) -> Result<Vec<ScoredDocument>> {
        let mut results = Vec::new();
        for hit in self.search(query, limit).await? {
            if let Some(document) = self.document_repo.find_by_id(hit.document_id).await? {
                results.push(ScoredDocument {
                    document,
                    score: hit.score,
                });
            }
        }
        Ok(results)
    }
}
