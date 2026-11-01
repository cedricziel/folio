//! Embeddings disabled

use std::sync::Arc;

use async_trait::async_trait;

use folio_core::error::{Error, Result};
use folio_core::ports::{EmbeddingBatch, EmbeddingService};

use crate::ai::registry::ProviderContext;

pub fn build(_: &ProviderContext) -> Result<Arc<dyn EmbeddingService>> {
    Ok(Arc::new(NoEmbeddings))
}

/// Refuses to embed; callers treat this as "semantic search is off"
pub struct NoEmbeddings;

#[async_trait]
impl EmbeddingService for NoEmbeddings {
    async fn embed(&self, _texts: &[&str]) -> Result<EmbeddingBatch> {
        Err(Error::AiService(
            "Embeddings are disabled (embeddings.provider = none)".to_string(),
        ))
    }

    async fn model_id(&self) -> Result<String> {
        Ok("none".to_string())
    }

    async fn health(&self) -> Result<()> {
        Ok(())
    }
}
