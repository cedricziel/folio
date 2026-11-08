//! Business services
//!
//! This module contains the core business logic that orchestrates
//! domain operations using the port traits.

mod file_sizes;
mod idempotent_uploads;
pub mod import;
mod importer;
mod ingest;
mod library;
mod outbox_dispatcher;
mod paperless_sources;
mod vaults;
mod workflow_engine;

pub use file_sizes::backfill_file_sizes;
pub use idempotent_uploads::{IdempotentUploads, UploadOutcome};
pub use importer::Importer;
pub use ingest::{content_key, IngestPipeline, IngestScheduler};
pub use library::{
    DocumentDetail, DocumentHit, DocumentQuery, LibraryService, MetadataUpdate, NewUpload,
    TextPage, VocabularyPolicy,
};
pub use outbox_dispatcher::{
    retry_delay, BroadcastConsumer, DispatcherConfig, EventConsumer, OutboxDispatcher,
};
pub use paperless_sources::{
    authorize_paperless, ensure_paperless_slot, split_shadowed_paperless, PAPERLESS_SLOT_TAKEN,
};
pub use vaults::{VaultService, PERSONAL_NAME};
pub use workflow_engine::WorkflowEngine;

use std::sync::Arc;

use serde::Serialize;

use crate::error::Result;
use crate::models::{CreateDocument, Document, DocumentFilter, Id, VaultId};
use crate::ports::{
    DocumentRepository, EmbeddedChunk, EmbeddingService, OcrService, SearchResult, SearchService,
};

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
    pub async fn get(&self, vault: VaultId, id: Id) -> Result<Option<Document>> {
        self.document_repo.find_by_id(vault, id).await
    }

    /// List documents with optional filtering
    pub async fn list(
        &self,
        vault: VaultId,
        filter: Option<DocumentFilter>,
    ) -> Result<Vec<Document>> {
        self.document_repo.list(vault, filter).await
    }

    /// Create a new document
    pub async fn create(&self, vault: VaultId, params: CreateDocument) -> Result<Document> {
        self.document_repo.create(vault, params).await
    }

    /// Delete a document
    pub async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        self.document_repo.delete(vault, id).await
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
        vault: VaultId,
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
            .create(
                vault,
                CreateDocument {
                    title,
                    content: Some(content.clone()),
                    original_filename: filename,
                    mime_type: Some(mime_type),
                    ..Default::default()
                },
            )
            .await?;

        self.embed(vault, doc.id, &content).await?;

        Ok(doc)
    }

    /// Re-process an existing document (regenerate OCR and embedding)
    pub async fn reprocess(
        &self,
        vault: VaultId,
        id: Id,
        data: &[u8],
        mime_type: &str,
    ) -> Result<Document> {
        // Get existing document
        let _existing = self
            .document_repo
            .find_by_id(vault, id)
            .await?
            .ok_or_else(|| crate::error::Error::NotFound {
                entity_type: "Document",
                id: id.to_string(),
            })?;

        // Extract text
        let content = self.ocr_service.extract_text(data, mime_type).await?;

        // Update document content
        let doc = self
            .document_repo
            .update(
                vault,
                id,
                crate::models::UpdateDocument {
                    content: Some(content.clone()),
                    ..Default::default()
                },
            )
            .await?;

        self.embed(vault, id, &content).await?;

        Ok(doc)
    }

    /// Embed the whole text as a single chunk and store it for search
    async fn embed(&self, vault: VaultId, id: Id, content: &str) -> Result<()> {
        if content.is_empty() {
            return Ok(());
        }
        let batch = self.embedding_service.embed(&[content]).await?;
        let chunks: Vec<EmbeddedChunk> = batch
            .vectors
            .into_iter()
            .enumerate()
            .map(|(i, embedding)| EmbeddedChunk {
                index: i as u32,
                text: content.to_string(),
                embedding,
            })
            .collect();
        self.search_service
            .store_chunks(vault, id, &batch.model, &chunks)
            .await
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
    pub async fn search(
        &self,
        vault: VaultId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let batch = self.embedding_service.embed(&[query]).await?;
        let vector = batch.vectors.first().map(Vec::as_slice).unwrap_or_default();
        self.search_service
            .search(vault, &batch.model, vector, limit)
            .await
    }

    /// Literal matches on title and text first, then semantic neighbours
    ///
    /// Semantic search is best effort: with embeddings disabled or failing, or
    /// before any document is embedded, literal matching still answers.
    pub async fn hybrid_search(
        &self,
        vault: VaultId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ScoredDocument>> {
        let semantic = self
            .search_documents(vault, query, limit)
            .await
            .unwrap_or_default();
        let scores: std::collections::HashMap<Id, f32> =
            semantic.iter().map(|s| (s.document.id, s.score)).collect();

        let literal = self
            .document_repo
            .list(
                vault,
                Some(DocumentFilter {
                    search_query: Some(query.to_string()),
                    ..Default::default()
                }),
            )
            .await?;

        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for document in literal {
            seen.insert(document.id);
            let score = scores.get(&document.id).copied().unwrap_or(1.0);
            out.push(ScoredDocument { document, score });
        }
        out.extend(semantic.into_iter().filter(|s| seen.insert(s.document.id)));
        out.truncate(limit);
        Ok(out)
    }

    /// Search by natural language query and resolve hits to full documents,
    /// skipping hits whose document no longer exists
    pub async fn search_documents(
        &self,
        vault: VaultId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ScoredDocument>> {
        let mut results = Vec::new();
        for hit in self.search(vault, query, limit).await? {
            if let Some(document) = self
                .document_repo
                .find_by_id(vault, hit.document_id)
                .await?
            {
                results.push(ScoredDocument {
                    document,
                    score: hit.score,
                });
            }
        }
        Ok(results)
    }
}
