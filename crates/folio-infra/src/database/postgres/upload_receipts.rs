use async_trait::async_trait;
use chrono::{DateTime, Utc};
use folio_core::error::{Error, Result};
use folio_core::models::Id;
use folio_core::ports::{
    UploadClaim, UploadReceiptKey, UploadReceiptRepository, UPLOAD_CLAIM_TIMEOUT,
    UPLOAD_RECEIPT_RETENTION,
};
use sqlx::PgPool;

#[derive(Clone)]
pub struct PostgresUploadReceiptRepository {
    pool: PgPool,
}

impl PostgresUploadReceiptRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn error(value: impl std::fmt::Display) -> Error {
    Error::Repository(value.to_string())
}

#[async_trait]
impl UploadReceiptRepository for PostgresUploadReceiptRepository {
    async fn claim(
        &self,
        key: &UploadReceiptKey,
        fingerprint: &str,
        now: DateTime<Utc>,
    ) -> Result<UploadClaim> {
        sqlx::query("DELETE FROM document_upload_receipts WHERE created_at < $1")
            .bind(now - UPLOAD_RECEIPT_RETENTION)
            .execute(&self.pool)
            .await
            .map_err(error)?;
        sqlx::query(
            "DELETE FROM document_upload_receipts WHERE document_id IS NULL AND created_at < $1",
        )
        .bind(now - UPLOAD_CLAIM_TIMEOUT)
        .execute(&self.pool)
        .await
        .map_err(error)?;
        let inserted = sqlx::query("INSERT INTO document_upload_receipts (vault_id, actor_scope, idempotency_key, request_fingerprint, created_at) VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING")
            .bind(key.vault.0).bind(&key.actor_scope).bind(key.key).bind(fingerprint).bind(now)
            .execute(&self.pool).await.map_err(error)?;
        if inserted.rows_affected() == 1 {
            return Ok(UploadClaim::Claimed);
        }
        let (previous, document): (String, Option<Id>) = sqlx::query_as("SELECT request_fingerprint, document_id FROM document_upload_receipts WHERE vault_id = $1 AND actor_scope = $2 AND idempotency_key = $3")
            .bind(key.vault.0).bind(&key.actor_scope).bind(key.key)
            .fetch_one(&self.pool).await.map_err(error)?;
        Ok(match document {
            _ if previous != fingerprint => UploadClaim::Mismatch,
            Some(id) => UploadClaim::Completed(id),
            None => UploadClaim::InProgress,
        })
    }

    async fn complete(&self, key: &UploadReceiptKey, document: Id) -> Result<()> {
        sqlx::query("UPDATE document_upload_receipts SET document_id = $1 WHERE vault_id = $2 AND actor_scope = $3 AND idempotency_key = $4")
            .bind(document).bind(key.vault.0).bind(&key.actor_scope).bind(key.key)
            .execute(&self.pool).await.map_err(error)?;
        Ok(())
    }

    async fn release(&self, key: &UploadReceiptKey) -> Result<()> {
        sqlx::query("DELETE FROM document_upload_receipts WHERE vault_id = $1 AND actor_scope = $2 AND idempotency_key = $3 AND document_id IS NULL")
            .bind(key.vault.0).bind(&key.actor_scope).bind(key.key)
            .execute(&self.pool).await.map_err(error)?;
        Ok(())
    }
}
