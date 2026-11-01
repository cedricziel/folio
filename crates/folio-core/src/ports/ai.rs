//! AI service port traits

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::models::Id;

/// Result of running an OCR engine over a document
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OcrOutput {
    /// Full extracted text; empty when the document has no recognisable text
    pub text: String,

    /// Text per page, when the engine works page by page
    pub pages: Vec<String>,

    /// Mean recognition confidence in `0.0..=1.0`, when the engine reports one
    pub confidence: Option<f32>,

    /// Identifier of the engine that produced the text (e.g. `tesseract`, `pdftotext`)
    pub engine: String,
}

impl OcrOutput {
    /// Build an output from per-page text, joining pages with a blank line
    pub fn from_pages(engine: impl Into<String>, pages: Vec<String>) -> Self {
        let text = pages
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        Self {
            text,
            pages,
            confidence: None,
            engine: engine.into(),
        }
    }
}

/// Service for extracting text from documents (OCR)
#[async_trait]
pub trait OcrService: Send + Sync {
    /// Extract text from a document
    ///
    /// # Arguments
    /// * `data` - Raw document bytes
    /// * `mime_type` - MIME type of the document (e.g., "image/png", "application/pdf")
    async fn extract(&self, data: &[u8], mime_type: &str) -> Result<OcrOutput>;

    /// Extract only the text, discarding page and confidence details
    async fn extract_text(&self, data: &[u8], mime_type: &str) -> Result<String> {
        Ok(self.extract(data, mime_type).await?.text)
    }

    /// Check that the engine is usable with the current settings
    async fn health(&self) -> Result<()>;
}

/// Vectors produced by one embedding call, tagged with the model that made them
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingBatch {
    /// One vector per input text, in input order
    pub vectors: Vec<Vec<f32>>,

    /// Identifier of the model (e.g. `openai:text-embedding-3-small`).
    /// Vectors are only comparable when this matches.
    pub model: String,

    /// Dimensionality of every vector in the batch
    pub dimensions: usize,
}

/// Service for generating vector embeddings
#[async_trait]
pub trait EmbeddingService: Send + Sync {
    /// Embed the given texts
    async fn embed(&self, texts: &[&str]) -> Result<EmbeddingBatch>;

    /// Identifier of the model that `embed` currently uses
    async fn model_id(&self) -> Result<String>;

    /// Check that the provider is reachable and the model is usable
    async fn health(&self) -> Result<()>;

    /// Generate an embedding vector for the given text
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        self.embed(&[text])
            .await?
            .vectors
            .into_iter()
            .next()
            .ok_or_else(|| crate::error::Error::AiService("No embedding returned".to_string()))
    }
}

/// A chunk of document text together with its vector
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddedChunk {
    /// Position of the chunk within the document
    pub index: u32,

    /// The chunk text
    pub text: String,

    /// The chunk vector
    pub embedding: Vec<f32>,
}

/// A search result with relevance score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Document ID
    pub document_id: Id,

    /// Relevance score (higher is more relevant)
    pub score: f32,

    /// Text of the best matching chunk
    pub snippet: Option<String>,
}

/// Service for semantic search
#[async_trait]
pub trait SearchService: Send + Sync {
    /// Replace all stored chunks of a document with freshly embedded ones
    async fn store_chunks(
        &self,
        document_id: Id,
        model: &str,
        chunks: &[EmbeddedChunk],
    ) -> Result<()>;

    /// Delete all stored chunks of a document
    async fn delete_embedding(&self, document_id: Id) -> Result<()>;

    /// Mark every vector produced by a model other than `active_model` as stale
    ///
    /// Returns the number of documents that now have no usable vectors.
    async fn mark_stale_except(&self, active_model: &str) -> Result<u64>;

    /// Search for documents by semantic similarity
    ///
    /// Only vectors produced by `model` are compared. Chunk scores are
    /// aggregated per document (best chunk wins).
    async fn search(
        &self,
        model: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>>;
}
