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
        let (document, token) = (Id::new_v4(), Id::new_v4());
        let inserted = sqlx::query("INSERT INTO document_upload_receipts (vault_id, actor_scope, idempotency_key, request_fingerprint, document_id, claim_token, claimed_at, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $7) ON CONFLICT DO NOTHING")
            .bind(key.vault.0).bind(&key.actor_scope).bind(key.key).bind(fingerprint).bind(document).bind(token).bind(now)
            .execute(&self.pool).await.map_err(error)?;
        if inserted.rows_affected() == 1 {
            return Ok(UploadClaim::Claimed { document, token });
        }
        let taken: Option<(Id,)> = sqlx::query_as("UPDATE document_upload_receipts SET claim_token = $1, claimed_at = $2 WHERE vault_id = $3 AND actor_scope = $4 AND idempotency_key = $5 AND request_fingerprint = $6 AND completed_at IS NULL AND claimed_at < $7 RETURNING document_id")
            .bind(token).bind(now).bind(key.vault.0).bind(&key.actor_scope).bind(key.key).bind(fingerprint).bind(now - UPLOAD_CLAIM_TIMEOUT)
            .fetch_optional(&self.pool).await.map_err(error)?;
        if let Some((document,)) = taken {
            return Ok(UploadClaim::Claimed { document, token });
        }
        let (previous, document, completed): (String, Id, Option<DateTime<Utc>>) = sqlx::query_as("SELECT request_fingerprint, document_id, completed_at FROM document_upload_receipts WHERE vault_id = $1 AND actor_scope = $2 AND idempotency_key = $3")
            .bind(key.vault.0).bind(&key.actor_scope).bind(key.key)
            .fetch_one(&self.pool).await.map_err(error)?;
        Ok(match completed {
            _ if previous != fingerprint => UploadClaim::Mismatch,
            Some(_) => UploadClaim::Completed(document),
            None => UploadClaim::InProgress(document),
        })
    }

    async fn complete(&self, key: &UploadReceiptKey, token: Id) -> Result<bool> {
        let updated = sqlx::query("UPDATE document_upload_receipts SET completed_at = now() WHERE vault_id = $1 AND actor_scope = $2 AND idempotency_key = $3 AND claim_token = $4 AND completed_at IS NULL")
            .bind(key.vault.0).bind(&key.actor_scope).bind(key.key).bind(token)
            .execute(&self.pool).await.map_err(error)?;
        Ok(updated.rows_affected() == 1)
    }

    async fn abandon(&self, key: &UploadReceiptKey, token: Id) -> Result<()> {
        sqlx::query("UPDATE document_upload_receipts SET claimed_at = 'epoch' WHERE vault_id = $1 AND actor_scope = $2 AND idempotency_key = $3 AND claim_token = $4 AND completed_at IS NULL")
            .bind(key.vault.0).bind(&key.actor_scope).bind(key.key).bind(token)
            .execute(&self.pool).await.map_err(error)?;
        Ok(())
    }
}
