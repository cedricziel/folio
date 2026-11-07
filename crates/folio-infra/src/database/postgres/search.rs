//! PostgreSQL semantic search store
//!
//! Chunk vectors are always stored in `document_chunks.embedding` (`real[]`).
//! When the `vector` extension (pgvector) is available, an extra
//! `embedding_vec` column is maintained and ranking happens in the database.
//! Without it, vectors are scored with cosine similarity in Rust.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;
use tracing::{info, instrument, warn};
use uuid::Uuid;

use folio_core::error::{Error, Result};
use folio_core::models::{Id, VaultId};
use folio_core::ports::{EmbeddedChunk, SearchResult, SearchService};

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
        sqlx::query("ALTER TABLE document_chunks ADD COLUMN IF NOT EXISTS embedding_vec vector")
            .execute(pool)
            .await?;
        // Rows written while the extension was missing.
        sqlx::query(
            "UPDATE document_chunks SET embedding_vec = embedding::vector
             WHERE embedding_vec IS NULL",
        )
        .execute(pool)
        .await?;
        Ok(true)
    }

    async fn search_pgvector(
        &self,
        vault: VaultId,
        model: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        // Best chunk per document; rows of another dimension cannot be compared.
        let rows: Vec<(Uuid, String, f64)> = sqlx::query_as(
            "SELECT document_id, text, score FROM (
                 SELECT DISTINCT ON (c.document_id) c.document_id, c.text,
                        1 - (c.embedding_vec <=> $1::vector) AS score
                 FROM document_chunks c JOIN documents d ON d.id = c.document_id
                 WHERE d.deleted_at IS NULL AND d.vault_id = $5 AND c.model = $2 AND NOT c.stale
                   AND c.embedding_vec IS NOT NULL AND vector_dims(c.embedding_vec) = $3
                 ORDER BY c.document_id, c.embedding_vec <=> $1::vector
             ) best
             ORDER BY score DESC
             LIMIT $4",
        )
        .bind(vector_literal(query_embedding))
        .bind(model)
        .bind(query_embedding.len() as i32)
        .bind(limit as i64)
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;

        Ok(rows
            .into_iter()
            .map(|(document_id, text, score)| SearchResult {
                document_id,
                score: score as f32,
                snippet: Some(text),
            })
            .collect())
    }

    async fn search_in_rust(
        &self,
        vault: VaultId,
        model: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let rows: Vec<(Uuid, String, Vec<f32>)> = sqlx::query_as(
            "SELECT c.document_id, c.text, c.embedding
             FROM document_chunks c JOIN documents d ON d.id = c.document_id
             WHERE d.deleted_at IS NULL AND d.vault_id = $2 AND c.model = $1 AND NOT c.stale",
        )
        .bind(model)
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;

        let mut best: HashMap<Id, SearchResult> = HashMap::new();
        for (document_id, text, embedding) in rows {
            let score = cosine_similarity(query_embedding, &embedding);
            let entry = best.entry(document_id).or_insert_with(|| SearchResult {
                document_id,
                score: f32::MIN,
                snippet: None,
            });
            if score > entry.score {
                entry.score = score;
                entry.snippet = Some(text);
            }
        }

        let mut results: Vec<SearchResult> = best.into_values().collect();
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
    async fn store_chunks(
        &self,
        vault: VaultId,
        document_id: Id,
        model: &str,
        chunks: &[EmbeddedChunk],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;

        let in_vault: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM documents WHERE deleted_at IS NULL AND id = $1 AND vault_id = $2",
        )
        .bind(document_id)
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?;
        if in_vault.is_none() {
            return Err(Error::NotFound {
                entity_type: "Document",
                id: document_id.to_string(),
            });
        }

        sqlx::query("DELETE FROM document_chunks WHERE document_id = $1")
            .bind(document_id)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;

        let now = Utc::now();
        for chunk in chunks {
            let query = if self.use_pgvector {
                sqlx::query(
                    "INSERT INTO document_chunks
                        (document_id, chunk_index, text, model, dimensions, embedding, stale,
                         created_at, embedding_vec)
                     VALUES ($1, $2, $3, $4, $5, $6, FALSE, $7, $8::vector)",
                )
            } else {
                sqlx::query(
                    "INSERT INTO document_chunks
                        (document_id, chunk_index, text, model, dimensions, embedding, stale,
                         created_at)
                     VALUES ($1, $2, $3, $4, $5, $6, FALSE, $7)",
                )
            };
            let query = query
                .bind(document_id)
                .bind(chunk.index as i32)
                .bind(&chunk.text)
                .bind(model)
                .bind(chunk.embedding.len() as i32)
                .bind(&chunk.embedding)
                .bind(now);
            let query = if self.use_pgvector {
                query.bind(vector_literal(&chunk.embedding))
            } else {
                query
            };
            query.execute(&mut *tx).await.map_err(repo_err)?;
        }

        tx.commit().await.map_err(repo_err)
    }

    async fn delete_embedding(&self, vault: VaultId, document_id: Id) -> Result<()> {
        sqlx::query(
            "DELETE FROM document_chunks WHERE document_id = $1
               AND document_id IN (SELECT id FROM documents WHERE deleted_at IS NULL AND vault_id = $2)",
        )
        .bind(document_id)
        .bind(vault.0)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn mark_stale_except(&self, active_model: &str) -> Result<u64> {
        sqlx::query(
            "UPDATE document_chunks SET stale = (model <> $1)
             WHERE stale IS DISTINCT FROM (model <> $1)",
        )
        .bind(active_model)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT document_id) FROM document_chunks WHERE stale",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(count as u64)
    }

    #[instrument(skip(self, query_embedding, limit), fields(limit = limit as i64))]
    async fn search(
        &self,
        vault: VaultId,
        model: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        if self.use_pgvector {
            self.search_pgvector(vault, model, query_embedding, limit)
                .await
        } else {
            self.search_in_rust(vault, model, query_embedding, limit)
                .await
        }
    }
}
