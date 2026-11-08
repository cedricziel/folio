use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use folio_core::error::{Error, Result};
use folio_core::models::Id;
use folio_core::ports::{
    UploadClaim, UploadReceiptKey, UploadReceiptRepository, UPLOAD_CLAIM_TIMEOUT,
    UPLOAD_RECEIPT_RETENTION,
};
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct SqliteUploadReceiptRepository {
    pool: SqlitePool,
}

impl SqliteUploadReceiptRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn error(value: impl std::fmt::Display) -> Error {
    Error::Repository(value.to_string())
}

/// Fixed-width UTC timestamps, so they compare correctly as text.
fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

#[async_trait]
impl UploadReceiptRepository for SqliteUploadReceiptRepository {
    async fn claim(
        &self,
        key: &UploadReceiptKey,
        fingerprint: &str,
        now: DateTime<Utc>,
    ) -> Result<UploadClaim> {
        sqlx::query("DELETE FROM document_upload_receipts WHERE created_at < ?")
            .bind(timestamp(now - UPLOAD_RECEIPT_RETENTION))
            .execute(&self.pool)
            .await
            .map_err(error)?;
        sqlx::query(
            "DELETE FROM document_upload_receipts WHERE document_id IS NULL AND created_at < ?",
        )
        .bind(timestamp(now - UPLOAD_CLAIM_TIMEOUT))
        .execute(&self.pool)
        .await
        .map_err(error)?;
        let inserted = sqlx::query("INSERT INTO document_upload_receipts (vault_id, actor_scope, idempotency_key, request_fingerprint, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO NOTHING")
            .bind(key.vault.to_string()).bind(&key.actor_scope).bind(key.key.to_string()).bind(fingerprint).bind(timestamp(now))
            .execute(&self.pool).await.map_err(error)?;
        if inserted.rows_affected() == 1 {
            return Ok(UploadClaim::Claimed);
        }
        let (previous, document): (String, Option<String>) = sqlx::query_as("SELECT request_fingerprint, document_id FROM document_upload_receipts WHERE vault_id = ? AND actor_scope = ? AND idempotency_key = ?")
            .bind(key.vault.to_string()).bind(&key.actor_scope).bind(key.key.to_string())
            .fetch_one(&self.pool).await.map_err(error)?;
        Ok(match document {
            _ if previous != fingerprint => UploadClaim::Mismatch,
            Some(id) => UploadClaim::Completed(id.parse::<Id>().map_err(error)?),
            None => UploadClaim::InProgress,
        })
    }

    async fn complete(&self, key: &UploadReceiptKey, document: Id) -> Result<()> {
        sqlx::query("UPDATE document_upload_receipts SET document_id = ? WHERE vault_id = ? AND actor_scope = ? AND idempotency_key = ?")
            .bind(document.to_string()).bind(key.vault.to_string()).bind(&key.actor_scope).bind(key.key.to_string())
            .execute(&self.pool).await.map_err(error)?;
        Ok(())
    }

    async fn release(&self, key: &UploadReceiptKey) -> Result<()> {
        sqlx::query("DELETE FROM document_upload_receipts WHERE vault_id = ? AND actor_scope = ? AND idempotency_key = ? AND document_id IS NULL")
            .bind(key.vault.to_string()).bind(&key.actor_scope).bind(key.key.to_string())
            .execute(&self.pool).await.map_err(error)?;
        Ok(())
    }
}
