use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use folio_core::error::{Error, Result};
use folio_core::models::{Id, VaultId};
use folio_core::ports::{
    ExpiredReservation, UploadClaim, UploadReceiptKey, UploadReceiptRepository,
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

const HELD: &str = "vault_id = ?2 AND actor_scope = ?3 AND idempotency_key = ?4 AND claim_token = ?5 AND completed_at IS NULL";

impl SqliteUploadReceiptRepository {
    /// Updates the uncompleted claim held with `token`; `set` refers to `now` as `?1`. True when it still held it.
    async fn update_held(
        &self,
        set: &str,
        now: DateTime<Utc>,
        key: &UploadReceiptKey,
        token: Id,
    ) -> Result<bool> {
        let sql = format!("UPDATE document_upload_receipts SET {set} WHERE {HELD}");
        let updated = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(timestamp(now))
            .bind(key.vault.to_string())
            .bind(&key.actor_scope)
            .bind(key.key.to_string())
            .bind(token.to_string())
            .execute(&self.pool)
            .await
            .map_err(error)?;
        Ok(updated.rows_affected() == 1)
    }
}

#[async_trait]
impl UploadReceiptRepository for SqliteUploadReceiptRepository {
    async fn claim(
        &self,
        key: &UploadReceiptKey,
        fingerprint: &str,
        now: DateTime<Utc>,
        stale_before: DateTime<Utc>,
    ) -> Result<UploadClaim> {
        let (document, token) = (Id::new_v4(), Id::new_v4());
        let inserted = sqlx::query("INSERT INTO document_upload_receipts (vault_id, actor_scope, idempotency_key, request_fingerprint, document_id, claim_token, last_active_at, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING")
            .bind(key.vault.to_string()).bind(&key.actor_scope).bind(key.key.to_string()).bind(fingerprint)
            .bind(document.to_string()).bind(token.to_string()).bind(timestamp(now)).bind(timestamp(now))
            .execute(&self.pool).await.map_err(error)?;
        if inserted.rows_affected() == 1 {
            return Ok(UploadClaim::Claimed {
                document,
                token,
                created: false,
            });
        }
        let taken: Option<(String, bool)> = sqlx::query_as("UPDATE document_upload_receipts SET claim_token = ?, last_active_at = ? WHERE vault_id = ? AND actor_scope = ? AND idempotency_key = ? AND request_fingerprint = ? AND completed_at IS NULL AND last_active_at < ? RETURNING document_id, document_created")
            .bind(token.to_string()).bind(timestamp(now)).bind(key.vault.to_string()).bind(&key.actor_scope)
            .bind(key.key.to_string()).bind(fingerprint).bind(timestamp(stale_before))
            .fetch_optional(&self.pool).await.map_err(error)?;
        if let Some((document, created)) = taken {
            return Ok(UploadClaim::Claimed {
                document: document.parse().map_err(error)?,
                token,
                created,
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

    async fn heartbeat(
        &self,
        key: &UploadReceiptKey,
        token: Id,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.update_held("last_active_at = ?1", now, key, token)
            .await
    }

    async fn mark_created(
        &self,
        key: &UploadReceiptKey,
        token: Id,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.update_held(
            "document_created = TRUE, last_active_at = ?1",
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
            "document_created = TRUE, completed_at = ?1, last_active_at = ?1",
            now,
            key,
            token,
        )
        .await
    }

    async fn complete_stored(&self, key: &UploadReceiptKey, now: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE document_upload_receipts SET document_created = TRUE, completed_at = ?1, last_active_at = ?1 WHERE vault_id = ?2 AND actor_scope = ?3 AND idempotency_key = ?4 AND completed_at IS NULL")
            .bind(timestamp(now)).bind(key.vault.to_string()).bind(&key.actor_scope).bind(key.key.to_string())
            .execute(&self.pool).await.map_err(error)?;
        Ok(())
    }

    async fn release(&self, key: &UploadReceiptKey, token: Id) -> Result<bool> {
        let deleted = sqlx::query("DELETE FROM document_upload_receipts WHERE vault_id = ?1 AND actor_scope = ?2 AND idempotency_key = ?3 AND claim_token = ?4 AND completed_at IS NULL")
        .bind(key.vault.to_string())
        .bind(&key.actor_scope)
        .bind(key.key.to_string())
        .bind(token.to_string())
        .execute(&self.pool)
        .await
        .map_err(error)?;
        Ok(deleted.rows_affected() == 1)
    }

    async fn expire(&self, before: DateTime<Utc>) -> Result<Vec<ExpiredReservation>> {
        let rows: Vec<(String, String, Option<String>)> = sqlx::query_as("DELETE FROM document_upload_receipts WHERE last_active_at < ? RETURNING vault_id, document_id, completed_at")
            .bind(timestamp(before)).fetch_all(&self.pool).await.map_err(error)?;
        rows.into_iter()
            .filter(|(_, _, completed)| completed.is_none())
            .map(|(vault, document, _)| {
                Ok(ExpiredReservation {
                    vault: VaultId(vault.parse().map_err(error)?),
                    document: document.parse().map_err(error)?,
                })
            })
            .collect()
    }
}
