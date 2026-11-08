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

/// Older than any claim, so an abandoned claim can be taken over at once.
const EPOCH: &str = "1970-01-01T00:00:00.000000Z";

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
        let (document, token) = (Id::new_v4(), Id::new_v4());
        let inserted = sqlx::query("INSERT INTO document_upload_receipts (vault_id, actor_scope, idempotency_key, request_fingerprint, document_id, claim_token, claimed_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING")
            .bind(key.vault.to_string()).bind(&key.actor_scope).bind(key.key.to_string()).bind(fingerprint)
            .bind(document.to_string()).bind(token.to_string()).bind(timestamp(now)).bind(timestamp(now))
            .execute(&self.pool).await.map_err(error)?;
        if inserted.rows_affected() == 1 {
            return Ok(UploadClaim::Claimed { document, token });
        }
        let taken: Option<(String,)> = sqlx::query_as("UPDATE document_upload_receipts SET claim_token = ?, claimed_at = ? WHERE vault_id = ? AND actor_scope = ? AND idempotency_key = ? AND request_fingerprint = ? AND completed_at IS NULL AND claimed_at < ? RETURNING document_id")
            .bind(token.to_string()).bind(timestamp(now)).bind(key.vault.to_string()).bind(&key.actor_scope)
            .bind(key.key.to_string()).bind(fingerprint).bind(timestamp(now - UPLOAD_CLAIM_TIMEOUT))
            .fetch_optional(&self.pool).await.map_err(error)?;
        if let Some((document,)) = taken {
            return Ok(UploadClaim::Claimed {
                document: document.parse().map_err(error)?,
                token,
            });
        }
        let (previous, document, completed): (String, String, Option<String>) = sqlx::query_as("SELECT request_fingerprint, document_id, completed_at FROM document_upload_receipts WHERE vault_id = ? AND actor_scope = ? AND idempotency_key = ?")
            .bind(key.vault.to_string()).bind(&key.actor_scope).bind(key.key.to_string())
            .fetch_one(&self.pool).await.map_err(error)?;
        let document: Id = document.parse().map_err(error)?;
        Ok(match completed {
            _ if previous != fingerprint => UploadClaim::Mismatch,
            Some(_) => UploadClaim::Completed(document),
            None => UploadClaim::InProgress(document),
        })
    }

    async fn complete(&self, key: &UploadReceiptKey, token: Id) -> Result<bool> {
        let updated = sqlx::query("UPDATE document_upload_receipts SET completed_at = ? WHERE vault_id = ? AND actor_scope = ? AND idempotency_key = ? AND claim_token = ? AND completed_at IS NULL")
            .bind(timestamp(Utc::now())).bind(key.vault.to_string()).bind(&key.actor_scope).bind(key.key.to_string()).bind(token.to_string())
            .execute(&self.pool).await.map_err(error)?;
        Ok(updated.rows_affected() == 1)
    }

    async fn abandon(&self, key: &UploadReceiptKey, token: Id) -> Result<()> {
        sqlx::query("UPDATE document_upload_receipts SET claimed_at = ? WHERE vault_id = ? AND actor_scope = ? AND idempotency_key = ? AND claim_token = ? AND completed_at IS NULL")
            .bind(EPOCH).bind(key.vault.to_string()).bind(&key.actor_scope).bind(key.key.to_string()).bind(token.to_string())
            .execute(&self.pool).await.map_err(error)?;
        Ok(())
    }
}
