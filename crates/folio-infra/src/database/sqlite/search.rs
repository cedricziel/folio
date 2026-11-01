//! Chunk-level vector storage and similarity search on SQLite

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::Utc;
use sqlx::SqlitePool;
use tracing::instrument;

use folio_core::error::{Error, Result};
use folio_core::models::Id;
use folio_core::ports::{EmbeddedChunk, SearchResult, SearchService};

use crate::ai::cosine_similarity;

fn db_error(e: sqlx::Error) -> Error {
    Error::Repository(e.to_string())
}

/// SQLite-backed search service; vectors are stored as little-endian f32 blobs
pub struct SqliteSearchService {
    pool: SqlitePool,
}

impl SqliteSearchService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn encode(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn decode(bytes: &[u8]) -> Vec<f32> {
    let (words, _) = bytes.as_chunks::<4>();
    words.iter().map(|w| f32::from_le_bytes(*w)).collect()
}

#[async_trait]
impl SearchService for SqliteSearchService {
    async fn store_chunks(
        &self,
        document_id: Id,
        model: &str,
        chunks: &[EmbeddedChunk],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(db_error)?;

        sqlx::query("DELETE FROM document_chunks WHERE document_id = ?")
            .bind(document_id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;

        let now = Utc::now().to_rfc3339();
        for chunk in chunks {
            sqlx::query(
                "INSERT INTO document_chunks
                    (document_id, chunk_index, text, model, dimensions, embedding, stale, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, 0, ?)",
            )
            .bind(document_id.to_string())
            .bind(chunk.index as i64)
            .bind(&chunk.text)
            .bind(model)
            .bind(chunk.embedding.len() as i64)
            .bind(encode(&chunk.embedding))
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        }

        tx.commit().await.map_err(db_error)
    }

    async fn delete_embedding(&self, document_id: Id) -> Result<()> {
        sqlx::query("DELETE FROM document_chunks WHERE document_id = ?")
            .bind(document_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn mark_stale_except(&self, active_model: &str) -> Result<u64> {
        sqlx::query(
            "UPDATE document_chunks
             SET stale = CASE WHEN model = ? THEN 0 ELSE 1 END
             WHERE stale != CASE WHEN model = ? THEN 0 ELSE 1 END",
        )
        .bind(active_model)
        .bind(active_model)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(DISTINCT document_id) FROM document_chunks WHERE stale = 1",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(count as u64)
    }

    #[instrument(skip(self, query_embedding))]
    async fn search(
        &self,
        model: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let rows: Vec<(String, String, Vec<u8>)> = sqlx::query_as(
            "SELECT document_id, text, embedding FROM document_chunks
             WHERE model = ? AND stale = 0",
        )
        .bind(model)
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)?;

        let mut best: HashMap<Id, SearchResult> = HashMap::new();
        for (doc_id, text, blob) in rows {
            let Ok(document_id) = Id::parse_str(&doc_id) else {
                continue;
            };
            let score = cosine_similarity(query_embedding, &decode(&blob));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_pool;
    use crate::SqliteDocumentRepository;
    use folio_core::models::CreateDocument;
    use folio_core::ports::DocumentRepository;

    async fn setup() -> (
        SqliteSearchService,
        SqliteDocumentRepository,
        tempfile::TempDir,
    ) {
        let (pool, dir) = test_pool().await;
        (
            SqliteSearchService::new(pool.clone()),
            SqliteDocumentRepository::new(pool),
            dir,
        )
    }

    async fn document(repo: &SqliteDocumentRepository, title: &str) -> Id {
        repo.create(CreateDocument {
            title: title.into(),
            content: None,
            original_filename: None,
            mime_type: None,
            ..Default::default()
        })
        .await
        .unwrap()
        .id
    }

    fn chunk(index: u32, text: &str, embedding: Vec<f32>) -> EmbeddedChunk {
        EmbeddedChunk {
            index,
            text: text.into(),
            embedding,
        }
    }

    #[tokio::test]
    async fn search_aggregates_chunks_per_document_with_best_snippet() {
        let (search, repo, _dir) = setup().await;
        let a = document(&repo, "a").await;
        let b = document(&repo, "b").await;
        search
            .store_chunks(
                a,
                "m1",
                &[
                    chunk(0, "unrelated", vec![0.0, 1.0]),
                    chunk(1, "the match", vec![1.0, 0.1]),
                ],
            )
            .await
            .unwrap();
        search
            .store_chunks(b, "m1", &[chunk(0, "weaker", vec![1.0, 1.0])])
            .await
            .unwrap();

        let results = search.search("m1", &[1.0, 0.0], 10).await.unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].document_id, a);
        assert_eq!(results[0].snippet.as_deref(), Some("the match"));
        assert!(results[0].score > results[1].score);
    }

    #[tokio::test]
    async fn search_only_compares_vectors_of_the_requested_model() {
        let (search, repo, _dir) = setup().await;
        let old = document(&repo, "old").await;
        let new = document(&repo, "new").await;
        search
            .store_chunks(old, "old-model", &[chunk(0, "x", vec![1.0, 0.0])])
            .await
            .unwrap();
        search
            .store_chunks(new, "new-model", &[chunk(0, "x", vec![1.0, 0.0])])
            .await
            .unwrap();

        let results = search.search("new-model", &[1.0, 0.0], 10).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].document_id, new);
    }

    #[tokio::test]
    async fn switching_models_marks_old_vectors_stale_and_switching_back_restores_them() {
        let (search, repo, _dir) = setup().await;
        let doc = document(&repo, "d").await;
        search
            .store_chunks(doc, "m1", &[chunk(0, "x", vec![1.0, 0.0])])
            .await
            .unwrap();

        assert_eq!(search.mark_stale_except("m2").await.unwrap(), 1);
        assert!(search
            .search("m1", &[1.0, 0.0], 5)
            .await
            .unwrap()
            .is_empty());

        assert_eq!(search.mark_stale_except("m1").await.unwrap(), 0);
        assert_eq!(search.search("m1", &[1.0, 0.0], 5).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn storing_replaces_previous_chunks_of_the_document() {
        let (search, repo, _dir) = setup().await;
        let doc = document(&repo, "d").await;
        search
            .store_chunks(
                doc,
                "m1",
                &[chunk(0, "a", vec![1.0]), chunk(1, "b", vec![1.0])],
            )
            .await
            .unwrap();
        search
            .store_chunks(doc, "m1", &[chunk(0, "only", vec![1.0])])
            .await
            .unwrap();

        let results = search.search("m1", &[1.0], 5).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].snippet.as_deref(), Some("only"));
    }

    #[test]
    fn vectors_round_trip_through_bytes() {
        let v = vec![0.25, -1.5, 3.0];
        assert_eq!(decode(&encode(&v)), v);
    }
}
