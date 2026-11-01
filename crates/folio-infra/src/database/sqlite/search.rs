//! SQLite semantic search store

use async_trait::async_trait;
use sqlx::SqlitePool;
use tracing::instrument;

use folio_core::error::{Error, Result};
use folio_core::models::Id;
use folio_core::ports::{SearchResult, SearchService};

use crate::ai::cosine_similarity;

/// SQLite-based search service storing embeddings as JSON
pub struct SqliteSearchService {
    pool: SqlitePool,
}

impl SqliteSearchService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SearchService for SqliteSearchService {
    async fn store_embedding(&self, document_id: Id, embedding: &[f32]) -> Result<()> {
        let embedding_json = serde_json::to_string(embedding)
            .map_err(|e| Error::AiService(format!("Failed to serialize embedding: {}", e)))?;

        sqlx::query(
            "INSERT OR REPLACE INTO document_embeddings (document_id, embedding) VALUES (?, ?)",
        )
        .bind(document_id.to_string())
        .bind(&embedding_json)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::Repository(e.to_string()))?;

        Ok(())
    }

    async fn delete_embedding(&self, document_id: Id) -> Result<()> {
        sqlx::query("DELETE FROM document_embeddings WHERE document_id = ?")
            .bind(document_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| Error::Repository(e.to_string()))?;

        Ok(())
    }

    #[instrument(skip(self, query_embedding))]
    async fn search(&self, query_embedding: &[f32], limit: usize) -> Result<Vec<SearchResult>> {
        // Fetch all embeddings (simple approach - for production, use a vector DB)
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT document_id, embedding FROM document_embeddings")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| Error::Repository(e.to_string()))?;

        let mut results: Vec<SearchResult> = rows
            .into_iter()
            .filter_map(|(doc_id, embedding_json)| {
                let embedding: Vec<f32> = serde_json::from_str(&embedding_json).ok()?;
                let score = cosine_similarity(query_embedding, &embedding);
                let document_id = uuid::Uuid::parse_str(&doc_id).ok()?;

                Some(SearchResult {
                    document_id,
                    score,
                    snippet: None,
                })
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);

        Ok(results)
    }
}
