//! PostgreSQL repositories for ingestion sources and the import ledger

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgRow, PgPool, Row};

use super::insert_domain_event;
use crate::database::source_lifecycle::source_event;
use folio_core::error::{Error, Result};
use folio_core::models::{
    Id, IngestionSourceDef, IngestionSourceInput, LedgerEntry, SourceLifecycle, SourceOrigin,
    SourceStatus, UserId, VaultId,
};
use folio_core::ports::{IngestLedger, SourceRepository};
use folio_core::services::PAPERLESS_SLOT_TAKEN;

fn repo_err(e: impl std::fmt::Display) -> Error {
    Error::Repository(e.to_string())
}

fn write_err(e: sqlx::Error, name: &str) -> Error {
    match e.as_database_error() {
        Some(db)
            if db.is_unique_violation()
                && db.constraint() == Some("ingestion_sources_one_paperless") =>
        {
            Error::Conflict(PAPERLESS_SLOT_TAKEN.into())
        }
        Some(db) if db.is_unique_violation() => Error::Duplicate {
            entity_type: "Source",
            field: "name",
            value: name.to_string(),
        },
        _ => repo_err(e),
    }
}

/// PostgreSQL-backed store for sources managed in the UI
#[derive(Clone)]
pub struct PostgresSourceRepository {
    pool: PgPool,
}

impl PostgresSourceRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn source_from_row(row: &PgRow) -> Result<IngestionSourceDef> {
    Ok(IngestionSourceDef {
        lifecycle: Some(SourceLifecycle {
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            deleted_at: row.get("deleted_at"),
            created_by: row.get::<Option<String>, _>("created_by").map(UserId),
            updated_by: row.get::<Option<String>, _>("updated_by").map(UserId),
        }),
        id: row.get("id"),
        name: row.get("name"),
        enabled: row.get("enabled"),
        origin: SourceOrigin::Managed,
        config: serde_json::from_str(row.get("config")).map_err(repo_err)?,
        vault_id: Some(VaultId(row.get("vault_id"))),
    })
}

/// Serialises Paperless writes per vault, then refuses a second live Paperless source
async fn claim_paperless(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    vault: VaultId,
    except: Option<Id>,
) -> Result<()> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('folio-paperless:' || $1::text, 0))",
    )
    .bind(vault.0)
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ingestion_sources WHERE vault_id = $1 AND kind = 'paperless' AND deleted_at IS NULL AND id IS DISTINCT FROM $2)",
    )
    .bind(vault.0)
    .bind(except)
    .fetch_one(&mut **tx)
    .await
    .map_err(repo_err)?;
    if taken {
        return Err(Error::Conflict(PAPERLESS_SLOT_TAKEN.into()));
    }
    Ok(())
}

#[async_trait]
impl SourceRepository for PostgresSourceRepository {
    async fn list(&self, vault: VaultId) -> Result<Vec<IngestionSourceDef>> {
        let rows = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE vault_id = $1 AND deleted_at IS NULL ORDER BY created_at, name",
        )
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(source_from_row).collect()
    }

    async fn list_all(&self) -> Result<Vec<IngestionSourceDef>> {
        let rows = sqlx::query(
            "SELECT s.* FROM ingestion_sources s JOIN vaults v ON v.id = s.vault_id
             WHERE s.deleted_at IS NULL AND v.deleted_at IS NULL ORDER BY s.created_at, s.name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(source_from_row).collect()
    }

    async fn find(&self, vault: VaultId, id: Id) -> Result<Option<IngestionSourceDef>> {
        let row = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE id = $1 AND vault_id = $2 AND deleted_at IS NULL",
        )
            .bind(id)
            .bind(vault.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(source_from_row).transpose()
    }

    async fn list_deleted(&self, vault: VaultId) -> Result<Vec<IngestionSourceDef>> {
        let rows = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE vault_id = $1 AND deleted_at IS NOT NULL ORDER BY deleted_at DESC, name",
        )
            .bind(vault.0).fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(source_from_row).collect()
    }

    async fn find_deleted(&self, vault: VaultId, id: Id) -> Result<Option<IngestionSourceDef>> {
        let row = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE id = $1 AND vault_id = $2 AND deleted_at IS NOT NULL",
        )
            .bind(id)
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(source_from_row).transpose()
    }

    async fn create(
        &self,
        vault: VaultId,
        input: IngestionSourceInput,
    ) -> Result<IngestionSourceDef> {
        input.validate()?;
        let id = Id::new_v4();
        let now = Utc::now();
        let actor = folio_core::mutation::current_actor();
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        if input.config.is_paperless() {
            claim_paperless(&mut tx, vault, None).await?;
        }
        let row = sqlx::query(
            "INSERT INTO ingestion_sources (id, vault_id, name, enabled, kind, config, created_at, updated_at, created_by, updated_by) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING *",
        )
        .bind(id)
        .bind(vault.0)
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.config.kind().as_str())
        .bind(serde_json::to_string(&input.config).map_err(repo_err)?)
        .bind(now)
        .bind(now)
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(actor.as_ref().map(UserId::as_str))
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| write_err(e, &input.name))?;
        let source = source_from_row(&row)?;
        insert_domain_event(&mut tx, &source_event(&source, "created")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(source)
    }

    async fn update(
        &self,
        vault: VaultId,
        id: Id,
        input: IngestionSourceInput,
    ) -> Result<IngestionSourceDef> {
        input.validate()?;
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE id = $1 AND vault_id = $2 AND deleted_at IS NULL FOR UPDATE",
        )
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or_else(|| Error::NotFound { entity_type: "Source", id: id.to_string() })?;
        let existing = source_from_row(&row)?;
        if existing.name == input.name
            && existing.enabled == input.enabled
            && existing.config == input.config
        {
            return Ok(existing);
        }
        if input.config.is_paperless() {
            claim_paperless(&mut tx, vault, Some(id)).await?;
        }
        let actor = folio_core::mutation::current_actor();
        let row = sqlx::query(
            "UPDATE ingestion_sources SET name = $1, enabled = $2, kind = $3, config = $4, updated_at = $5, updated_by = $6 WHERE id = $7 AND vault_id = $8 RETURNING *",
        )
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.config.kind().as_str())
        .bind(serde_json::to_string(&input.config).map_err(repo_err)?)
        .bind(Utc::now())
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id)
        .bind(vault.0)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| write_err(e, &input.name))?;
        let source = source_from_row(&row)?;
        insert_domain_event(&mut tx, &source_event(&source, "updated")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(source)
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE id = $1 AND vault_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or_else(|| Error::NotFound {
            entity_type: "Source",
            id: id.to_string(),
        })?;
        let existing = source_from_row(&row)?;
        if existing
            .lifecycle
            .as_ref()
            .and_then(|m| m.deleted_at)
            .is_some()
        {
            return Ok(());
        }
        let actor = folio_core::mutation::current_actor();
        let now = Utc::now();
        let row = sqlx::query(
            "UPDATE ingestion_sources SET deleted_at = $1, updated_at = $2, updated_by = $3 WHERE id = $4 AND vault_id = $5 RETURNING *",
        )
        .bind(Some(now))
        .bind(now)
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id)
        .bind(vault.0)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| write_err(e, &existing.name))?;
        let source = source_from_row(&row)?;
        insert_domain_event(&mut tx, &source_event(&source, "deleted")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }

    async fn restore(&self, vault: VaultId, id: Id) -> Result<IngestionSourceDef> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE id = $1 AND vault_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(vault.0)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or_else(|| Error::NotFound {
            entity_type: "Source",
            id: id.to_string(),
        })?;
        let existing = source_from_row(&row)?;
        if existing
            .lifecycle
            .as_ref()
            .and_then(|m| m.deleted_at)
            .is_none()
        {
            return Ok(existing);
        }
        if existing.config.is_paperless() {
            claim_paperless(&mut tx, vault, Some(id)).await?;
        }
        let actor = folio_core::mutation::current_actor();
        let now = Utc::now();
        let row = sqlx::query(
            "UPDATE ingestion_sources SET deleted_at = $1, updated_at = $2, updated_by = $3 WHERE id = $4 AND vault_id = $5 RETURNING *",
        )
        .bind(None::<DateTime<Utc>>)
        .bind(now)
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id)
        .bind(vault.0)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| write_err(e, &existing.name))?;
        let source = source_from_row(&row)?;
        insert_domain_event(&mut tx, &source_event(&source, "restored")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(source)
    }

    async fn save_status(&self, id: Id, status: &SourceStatus) -> Result<()> {
        sqlx::query(
            "INSERT INTO ingestion_status
                 (source_id, last_run_at, last_error, last_imported, last_skipped, last_failed)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT(source_id) DO UPDATE SET
                 last_run_at = excluded.last_run_at,
                 last_error = excluded.last_error,
                 last_imported = excluded.last_imported,
                 last_skipped = excluded.last_skipped,
                 last_failed = excluded.last_failed",
        )
        .bind(id)
        .bind(status.last_run_at)
        .bind(&status.last_error)
        .bind(status.last_imported as i32)
        .bind(status.last_skipped as i32)
        .bind(status.last_failed as i32)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn status(&self, id: Id) -> Result<Option<SourceStatus>> {
        let row = sqlx::query("SELECT * FROM ingestion_status WHERE source_id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(row.map(|row| SourceStatus {
            last_run_at: row.get::<Option<DateTime<Utc>>, _>("last_run_at"),
            last_error: row.get("last_error"),
            last_imported: row.get::<i32, _>("last_imported") as u32,
            last_skipped: row.get::<i32, _>("last_skipped") as u32,
            last_failed: row.get::<i32, _>("last_failed") as u32,
        }))
    }
}

/// PostgreSQL-backed import ledger
#[derive(Clone)]
pub struct PostgresIngestLedger {
    pool: PgPool,
}

impl PostgresIngestLedger {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

async fn ensure_source_active(
    conn: &mut sqlx::PgConnection,
    vault: VaultId,
    source_id: Id,
) -> Result<()> {
    // Configured sources have no persisted row.
    let blocked: Option<bool> = sqlx::query_scalar("SELECT deleted_at IS NOT NULL OR vault_id != $1 FROM ingestion_sources WHERE id = $2 FOR SHARE")
        .bind(vault.0)
        .bind(source_id)
        .fetch_optional(conn)
        .await
        .map_err(repo_err)?;
    if blocked == Some(true) {
        return Err(Error::Conflict(
            "This ingestion source is deleted or belongs to a different vault".into(),
        ));
    }
    Ok(())
}

#[async_trait]
impl IngestLedger for PostgresIngestLedger {
    async fn insert(&self, vault: VaultId, entry: &LedgerEntry) -> Result<bool> {
        if entry.vault_id != vault {
            return Err(Error::Validation(
                "Ingest entry belongs to a different vault".into(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        ensure_source_active(&mut tx, vault, entry.source_id).await?;
        let result = sqlx::query(
            "INSERT INTO ingest_ledger (vault_id, key, source_id, document_id, outcome, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT(vault_id, key) DO NOTHING",
        )
        .bind(vault.0)
        .bind(&entry.key)
        .bind(entry.source_id)
        .bind(entry.document_id)
        .bind(&entry.outcome)
        .bind(entry.created_at)
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        tx.commit().await.map_err(repo_err)?;
        Ok(result.rows_affected() == 1)
    }

    async fn find(&self, vault: VaultId, key: &str) -> Result<Option<LedgerEntry>> {
        let row = sqlx::query("SELECT * FROM ingest_ledger WHERE vault_id = $1 AND key = $2")
            .bind(vault.0)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(row.map(|row| LedgerEntry {
            vault_id: VaultId(row.get("vault_id")),
            key: row.get("key"),
            source_id: row.get("source_id"),
            document_id: row.get("document_id"),
            outcome: row.get("outcome"),
            created_at: row.get("created_at"),
        }))
    }

    async fn complete(
        &self,
        vault: VaultId,
        key: &str,
        outcome: &str,
        document_id: Option<Id>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE ingest_ledger SET outcome = $1, document_id = $2
             WHERE vault_id = $3 AND key = $4",
        )
        .bind(outcome)
        .bind(document_id)
        .bind(vault.0)
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn transition_if_matches(
        &self,
        vault: VaultId,
        expected: &LedgerEntry,
        outcome: &str,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        if expected.vault_id != vault {
            return Err(Error::Validation(
                "Ingest entry belongs to a different vault".into(),
            ));
        }
        let result = sqlx::query(
            "UPDATE ingest_ledger SET outcome = $1, created_at = $2
             WHERE vault_id = $3 AND key = $4 AND source_id = $5
               AND document_id IS NOT DISTINCT FROM $6 AND outcome = $7 AND created_at = $8",
        )
        .bind(outcome)
        .bind(created_at)
        .bind(vault.0)
        .bind(&expected.key)
        .bind(expected.source_id)
        .bind(expected.document_id)
        .bind(&expected.outcome)
        .bind(expected.created_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() == 1)
    }

    async fn delete_if_matches(&self, vault: VaultId, expected: &LedgerEntry) -> Result<bool> {
        if expected.vault_id != vault {
            return Err(Error::Validation(
                "Ingest entry belongs to a different vault".into(),
            ));
        }
        let result = sqlx::query(
            "DELETE FROM ingest_ledger
             WHERE vault_id = $1 AND key = $2 AND source_id = $3
               AND document_id IS NOT DISTINCT FROM $4 AND outcome = $5 AND created_at = $6",
        )
        .bind(vault.0)
        .bind(&expected.key)
        .bind(expected.source_id)
        .bind(expected.document_id)
        .bind(&expected.outcome)
        .bind(expected.created_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() == 1)
    }

    async fn delete(&self, vault: VaultId, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM ingest_ledger WHERE vault_id = $1 AND key = $2")
            .bind(vault.0)
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}
