//! AI service port traits

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::models::Id;

/// Service for extracting text from documents (OCR)
#[async_trait]
pub trait OcrService: Send + Sync {
    /// Extract text from a document
    ///
    /// # Arguments
    /// * `data` - Raw document bytes
    /// * `mime_type` - MIME type of the document (e.g., "image/png", "application/pdf")
    ///
    /// # Returns
    /// Extracted text content
    async fn extract_text(&self, data: &[u8], mime_type: &str) -> Result<String>;
}

/// Service for generating vector embeddings
#[async_trait]
pub trait EmbeddingService: Send + Sync {
    /// Generate an embedding vector for the given text
    ///
    /// # Arguments
    /// * `text` - Text to embed
    ///
    /// # Returns
    /// Embedding vector as a Vec<f32>
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>>;

    /// Generate embeddings for multiple texts in batch
    ///
    /// # Arguments
    /// * `texts` - Texts to embed
    ///
    /// # Returns
    /// Embedding vectors for each input text
    async fn generate_embeddings(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;

    /// Get the dimensionality of the embedding vectors
    fn embedding_dimension(&self) -> usize;
}

/// A search result with relevance score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Document ID
    pub document_id: Id,

    /// Relevance score (higher is more relevant)
    pub score: f32,

    /// Optional snippet of matching content
    pub snippet: Option<String>,
}

/// Service for semantic search
#[async_trait]
pub trait SearchService: Send + Sync {
    /// Store an embedding for a document
    async fn store_embedding(&self, document_id: Id, embedding: &[f32]) -> Result<()>;

    /// Delete the embedding for a document
    async fn delete_embedding(&self, document_id: Id) -> Result<()>;

    /// Search for documents by semantic similarity
    ///
    /// # Arguments
    /// * `query_embedding` - Embedding vector of the search query
    /// * `limit` - Maximum number of results to return
    ///
    /// # Returns
    /// Search results ordered by relevance (most relevant first)
    async fn search(&self, query_embedding: &[f32], limit: usize) -> Result<Vec<SearchResult>>;
}
