//! PostgreSQL semantic search store
//!
//! Embeddings are always stored in `document_embeddings.embedding` (`real[]`).
//! When the `vector` extension (pgvector) is available, an extra
//! `embedding_vec` column is maintained and ranking happens in the database.
//! Without it, vectors are scored with cosine similarity in Rust.

use async_trait::async_trait;
use sqlx::PgPool;
use tracing::{info, instrument, warn};
use uuid::Uuid;

use folio_core::error::{Error, Result};
use folio_core::models::Id;
use folio_core::ports::{SearchResult, SearchService};

use crate::ai::cosine_similarity;

pub struct PostgresSearchService {
    pool: PgPool,
    use_pgvector: bool,
}

fn repo_err(e: sqlx::Error) -> Error {
    Error::Repository(e.to_string())
}

/// pgvector text literal, e.g. `[0.1,0.2]`.
fn vector_literal(embedding: &[f32]) -> String {
    let parts: Vec<String> = embedding.iter().map(|v| v.to_string()).collect();
    format!("[{}]", parts.join(","))
}

impl PostgresSearchService {
    /// Create the service, enabling pgvector ranking when the extension can be
    /// installed or is already present.
    pub async fn new(pool: PgPool) -> Self {
        let use_pgvector = match Self::enable_pgvector(&pool).await {
            Ok(enabled) => enabled,
            Err(e) => {
                warn!("pgvector setup failed, using Rust cosine search: {e}");
                false
            }
        };
        if use_pgvector {
            info!("Semantic search uses pgvector");
        } else {
            info!("pgvector not available, semantic search uses Rust cosine similarity");
        }
        Self { pool, use_pgvector }
    }

    /// Skip pgvector even when installed.
    pub fn without_pgvector(pool: PgPool) -> Self {
        Self {
            pool,
            use_pgvector: false,
        }
    }

    pub fn uses_pgvector(&self) -> bool {
        self.use_pgvector
    }

    async fn enable_pgvector(pool: &PgPool) -> std::result::Result<bool, sqlx::Error> {
        let available: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'vector')",
        )
        .fetch_one(pool)
        .await?;
        if !available {
            return Ok(false);
        }
        sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
            .execute(pool)
            .await?;
        sqlx::query(
            "ALTER TABLE document_embeddings ADD COLUMN IF NOT EXISTS embedding_vec vector",
        )
        .execute(pool)
        .await?;
        // Rows written while the extension was missing.
        sqlx::query(
            "UPDATE document_embeddings SET embedding_vec = embedding::vector
             WHERE embedding_vec IS NULL",
        )
        .execute(pool)
        .await?;
        Ok(true)
    }

    async fn search_pgvector(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        // Rows with a different dimension (for example after switching models)
        // cannot be compared, so they are skipped.
        let rows: Vec<(Uuid, f64)> = sqlx::query_as(
            "SELECT document_id, 1 - (embedding_vec <=> $1::vector) AS score
             FROM document_embeddings
             WHERE embedding_vec IS NOT NULL AND vector_dims(embedding_vec) = $2
             ORDER BY embedding_vec <=> $1::vector
             LIMIT $3",
        )
        .bind(vector_literal(query_embedding))
        .bind(query_embedding.len() as i32)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;

        Ok(rows
            .into_iter()
            .map(|(document_id, score)| SearchResult {
                document_id,
                score: score as f32,
                snippet: None,
            })
            .collect())
    }

    async fn search_in_rust(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let rows: Vec<(Uuid, Vec<f32>)> =
            sqlx::query_as("SELECT document_id, embedding FROM document_embeddings")
                .fetch_all(&self.pool)
                .await
                .map_err(repo_err)?;

        let mut results: Vec<SearchResult> = rows
            .into_iter()
            .map(|(document_id, embedding)| SearchResult {
                document_id,
                score: cosine_similarity(query_embedding, &embedding),
                snippet: None,
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

#[async_trait]
impl SearchService for PostgresSearchService {
    async fn store_embedding(&self, document_id: Id, embedding: &[f32]) -> Result<()> {
        if self.use_pgvector {
            sqlx::query(
                "INSERT INTO document_embeddings (document_id, embedding, embedding_vec)
                 VALUES ($1, $2, $3::vector)
                 ON CONFLICT (document_id) DO UPDATE
                 SET embedding = EXCLUDED.embedding, embedding_vec = EXCLUDED.embedding_vec",
            )
            .bind(document_id)
            .bind(embedding)
            .bind(vector_literal(embedding))
            .execute(&self.pool)
            .await
        } else {
            sqlx::query(
                "INSERT INTO document_embeddings (document_id, embedding) VALUES ($1, $2)
                 ON CONFLICT (document_id) DO UPDATE SET embedding = EXCLUDED.embedding",
            )
            .bind(document_id)
            .bind(embedding)
            .execute(&self.pool)
            .await
        }
        .map_err(repo_err)?;

        Ok(())
    }

    async fn delete_embedding(&self, document_id: Id) -> Result<()> {
        sqlx::query("DELETE FROM document_embeddings WHERE document_id = $1")
            .bind(document_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    #[instrument(skip(self, query_embedding))]
    async fn search(&self, query_embedding: &[f32], limit: usize) -> Result<Vec<SearchResult>> {
        if self.use_pgvector {
            self.search_pgvector(query_embedding, limit).await
        } else {
            self.search_in_rust(query_embedding, limit).await
        }
    }
}
