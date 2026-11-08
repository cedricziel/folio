use async_trait::async_trait;
use chrono::{DateTime, Utc};
use folio_core::error::{Error, Result};
use folio_core::models::{Id, VaultId};
use folio_core::ports::{
    ExpiredReservation, UploadClaim, UploadReceiptKey, UploadReceiptRepository,
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

const HELD: &str = "vault_id = $2 AND actor_scope = $3 AND idempotency_key = $4 AND claim_token = $5 AND completed_at IS NULL";

impl PostgresUploadReceiptRepository {
    /// Updates the uncompleted claim held with `token` (`$1` is `now`); true when it still held it.
    async fn update_held(
        &self,
        set: &str,
        now: DateTime<Utc>,
        key: &UploadReceiptKey,
        token: Id,
    ) -> Result<bool> {
        let updated = sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE document_upload_receipts SET {set} WHERE {HELD}"
        )))
        .bind(now)
        .bind(key.vault.0)
        .bind(&key.actor_scope)
        .bind(key.key)
        .bind(token)
        .execute(&self.pool)
        .await
        .map_err(error)?;
        Ok(updated.rows_affected() == 1)
    }
}

#[async_trait]
impl UploadReceiptRepository for PostgresUploadReceiptRepository {
    async fn claim(
        &self,
        key: &UploadReceiptKey,
        fingerprint: &str,
        now: DateTime<Utc>,
        stale_before: DateTime<Utc>,
    ) -> Result<UploadClaim> {
        let (document, token) = (Id::new_v4(), Id::new_v4());
        let inserted = sqlx::query("INSERT INTO document_upload_receipts (vault_id, actor_scope, idempotency_key, request_fingerprint, document_id, claim_token, last_active_at, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $7) ON CONFLICT DO NOTHING")
            .bind(key.vault.0).bind(&key.actor_scope).bind(key.key).bind(fingerprint).bind(document).bind(token).bind(now)
            .execute(&self.pool).await.map_err(error)?;
        if inserted.rows_affected() == 1 {
            return Ok(UploadClaim::Claimed {
                document,
                token,
                created: false,
            });
        }
        let taken: Option<(Id, bool)> = sqlx::query_as("UPDATE document_upload_receipts SET claim_token = $1, last_active_at = $2 WHERE vault_id = $3 AND actor_scope = $4 AND idempotency_key = $5 AND request_fingerprint = $6 AND completed_at IS NULL AND last_active_at < $7 RETURNING document_id, document_created")
            .bind(token).bind(now).bind(key.vault.0).bind(&key.actor_scope).bind(key.key).bind(fingerprint).bind(stale_before)
            .fetch_optional(&self.pool).await.map_err(error)?;
        if let Some((document, created)) = taken {
            return Ok(UploadClaim::Claimed {
                document,
                token,
                created,
            });
        }
        let (previous, document, completed): (String, Id, Option<DateTime<Utc>>) = sqlx::query_as("UPDATE document_upload_receipts SET last_active_at = CASE WHEN completed_at IS NOT NULL AND request_fingerprint = $4 THEN GREATEST(last_active_at, $5) ELSE last_active_at END WHERE vault_id = $1 AND actor_scope = $2 AND idempotency_key = $3 RETURNING request_fingerprint, document_id, completed_at")
            .bind(key.vault.0).bind(&key.actor_scope).bind(key.key).bind(fingerprint).bind(now)
            .fetch_one(&self.pool).await.map_err(error)?;
        Ok(match completed {
            _ if previous != fingerprint => UploadClaim::Mismatch,
            Some(_) => UploadClaim::Completed(document),
            None => UploadClaim::InProgress(document),
        })
    }

    async fn heartbeat(
        &self,
        key: &UploadReceiptKey,
        token: Id,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.update_held("last_active_at = $1", now, key, token)
            .await
    }

    async fn mark_created(
        &self,
        key: &UploadReceiptKey,
        token: Id,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.update_held(
            "document_created = TRUE, last_active_at = $1",
            now,
            key,
            token,
        )
        .await
    }

    async fn complete(
        &self,
        key: &UploadReceiptKey,
        token: Id,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.update_held(
            "document_created = TRUE, completed_at = $1, last_active_at = $1",
            now,
            key,
            token,
        )
        .await
    }

    async fn complete_stored(&self, key: &UploadReceiptKey, now: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE document_upload_receipts SET document_created = TRUE, completed_at = $1, last_active_at = $1 WHERE vault_id = $2 AND actor_scope = $3 AND idempotency_key = $4 AND completed_at IS NULL")
            .bind(now).bind(key.vault.0).bind(&key.actor_scope).bind(key.key)
            .execute(&self.pool).await.map_err(error)?;
        Ok(())
    }

    async fn release(&self, key: &UploadReceiptKey, token: Id) -> Result<bool> {
        let deleted = sqlx::query("DELETE FROM document_upload_receipts WHERE vault_id = $1 AND actor_scope = $2 AND idempotency_key = $3 AND claim_token = $4 AND completed_at IS NULL")
            .bind(key.vault.0).bind(&key.actor_scope).bind(key.key).bind(token)
            .execute(&self.pool).await.map_err(error)?;
        Ok(deleted.rows_affected() == 1)
    }

    async fn expire(&self, before: DateTime<Utc>) -> Result<Vec<ExpiredReservation>> {
        let rows: Vec<(Id, Id, Option<DateTime<Utc>>)> = sqlx::query_as("DELETE FROM document_upload_receipts WHERE last_active_at < $1 RETURNING vault_id, document_id, completed_at")
            .bind(before).fetch_all(&self.pool).await.map_err(error)?;
        Ok(rows
            .into_iter()
            .filter(|(_, _, completed)| completed.is_none())
            .map(|(vault, document, _)| ExpiredReservation {
                vault: VaultId(vault),
                document,
            })
            .collect())
    }
}
