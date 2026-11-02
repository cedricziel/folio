//! SQLite repositories for ingestion sources and the import ledger

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{sqlite::SqliteRow, Row, SqlitePool};

use super::insert_domain_event;
use crate::database::source_lifecycle::source_event;
use folio_core::error::{Error, Result};
use folio_core::models::{
    Id, IngestionSourceDef, IngestionSourceInput, LedgerEntry, SourceLifecycle, SourceOrigin,
    SourceStatus, UserId, VaultId,
};
use folio_core::ports::{IngestLedger, SourceRepository};

fn repo_err(e: impl std::fmt::Display) -> Error {
    Error::Repository(e.to_string())
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(repo_err)
}

fn parse_id(value: &str) -> Result<Id> {
    Id::parse_str(value).map_err(repo_err)
}

/// SQLite-backed store for sources managed in the UI
#[derive(Clone)]
pub struct SqliteSourceRepository {
    pool: SqlitePool,
}

impl SqliteSourceRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn source_from_row(row: &SqliteRow) -> Result<IngestionSourceDef> {
    Ok(IngestionSourceDef {
        lifecycle: Some(SourceLifecycle {
            created_at: parse_time(row.get("created_at"))?,
            updated_at: parse_time(row.get("updated_at"))?,
            deleted_at: row
                .get::<Option<&str>, _>("deleted_at")
                .map(parse_time)
                .transpose()?,
            created_by: row.get::<Option<String>, _>("created_by").map(UserId),
            updated_by: row.get::<Option<String>, _>("updated_by").map(UserId),
        }),
        id: parse_id(row.get("id"))?,
        name: row.get("name"),
        enabled: row.get::<i64, _>("enabled") != 0,
        origin: SourceOrigin::Managed,
        config: serde_json::from_str(row.get("config")).map_err(repo_err)?,
        vault_id: Some(VaultId(parse_id(row.get("vault_id"))?)),
    })
}

fn write_err(e: sqlx::Error, name: &str) -> Error {
    match &e {
        sqlx::Error::Database(db) if db.is_unique_violation() => Error::Duplicate {
            entity_type: "Source",
            field: "name",
            value: name.to_string(),
        },
        _ => repo_err(e),
    }
}

#[async_trait]
impl SourceRepository for SqliteSourceRepository {
    async fn list(&self, vault: VaultId) -> Result<Vec<IngestionSourceDef>> {
        let rows = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE vault_id = ? AND deleted_at IS NULL ORDER BY created_at, name",
        )
        .bind(vault.to_string())
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(source_from_row).collect()
    }

    async fn list_all(&self) -> Result<Vec<IngestionSourceDef>> {
        let rows = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE deleted_at IS NULL ORDER BY created_at, name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(source_from_row).collect()
    }

    async fn find(&self, vault: VaultId, id: Id) -> Result<Option<IngestionSourceDef>> {
        let row = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE id = ? AND vault_id = ? AND deleted_at IS NULL",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(source_from_row).transpose()
    }

    async fn list_deleted(&self, vault: VaultId) -> Result<Vec<IngestionSourceDef>> {
        let rows = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE vault_id = ? AND deleted_at IS NOT NULL ORDER BY deleted_at DESC, name",
        )
            .bind(vault.to_string()).fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(source_from_row).collect()
    }

    async fn find_deleted(&self, vault: VaultId, id: Id) -> Result<Option<IngestionSourceDef>> {
        let row = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE id = ? AND vault_id = ? AND deleted_at IS NOT NULL",
        )
            .bind(id.to_string())
        .bind(vault.to_string())
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
        let now = Utc::now().to_rfc3339();
        let actor = folio_core::mutation::current_actor();
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        let row = sqlx::query(
            "INSERT INTO ingestion_sources (id, vault_id, name, enabled, kind, config, created_at, updated_at, created_by, updated_by) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.config.kind().as_str())
        .bind(serde_json::to_string(&input.config).map_err(repo_err)?)
        .bind(&now)
        .bind(&now)
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
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        let row = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE id = ? AND vault_id = ? AND deleted_at IS NULL",
        )
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_err)?
        .ok_or_else(|| Error::NotFound {
            entity_type: "Source",
            id: id.to_string(),
        })?;
        let existing = source_from_row(&row)?;
        if existing.name == input.name
            && existing.enabled == input.enabled
            && existing.config == input.config
        {
            return Ok(existing);
        }
        let actor = folio_core::mutation::current_actor();
        let row = sqlx::query(
            "UPDATE ingestion_sources SET name = ?, enabled = ?, kind = ?, config = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? RETURNING *",
        )
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.config.kind().as_str())
        .bind(serde_json::to_string(&input.config).map_err(repo_err)?)
        .bind(Utc::now().to_rfc3339())
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| write_err(e, &input.name))?;
        let source = source_from_row(&row)?;
        insert_domain_event(&mut tx, &source_event(&source, "updated")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(source)
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        let row = sqlx::query("SELECT * FROM ingestion_sources WHERE id = ? AND vault_id = ?")
            .bind(id.to_string())
            .bind(vault.to_string())
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
        let now = Utc::now().to_rfc3339();
        let row = sqlx::query(
            "UPDATE ingestion_sources SET deleted_at = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? RETURNING *",
        )
        .bind(Some(&now))
        .bind(&now)
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id.to_string())
        .bind(vault.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| write_err(e, &existing.name))?;
        let source = source_from_row(&row)?;
        insert_domain_event(&mut tx, &source_event(&source, "deleted")?).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }

    async fn restore(&self, vault: VaultId, id: Id) -> Result<IngestionSourceDef> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        let row = sqlx::query("SELECT * FROM ingestion_sources WHERE id = ? AND vault_id = ?")
            .bind(id.to_string())
            .bind(vault.to_string())
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
        let actor = folio_core::mutation::current_actor();
        let now = Utc::now().to_rfc3339();
        let row = sqlx::query(
            "UPDATE ingestion_sources SET deleted_at = ?, updated_at = ?, updated_by = ? WHERE id = ? AND vault_id = ? RETURNING *",
        )
        .bind(None::<String>)
        .bind(&now)
        .bind(actor.as_ref().map(UserId::as_str))
        .bind(id.to_string())
        .bind(vault.to_string())
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
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(source_id) DO UPDATE SET
                 last_run_at = excluded.last_run_at,
                 last_error = excluded.last_error,
                 last_imported = excluded.last_imported,
                 last_skipped = excluded.last_skipped,
                 last_failed = excluded.last_failed",
        )
        .bind(id.to_string())
        .bind(status.last_run_at.map(|t| t.to_rfc3339()))
        .bind(&status.last_error)
        .bind(status.last_imported)
        .bind(status.last_skipped)
        .bind(status.last_failed)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn status(&self, id: Id) -> Result<Option<SourceStatus>> {
        let row = sqlx::query("SELECT * FROM ingestion_status WHERE source_id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.map(|row| {
            let last_run_at: Option<&str> = row.get("last_run_at");
            Ok(SourceStatus {
                last_run_at: last_run_at.map(parse_time).transpose()?,
                last_error: row.get("last_error"),
                last_imported: row.get::<i64, _>("last_imported") as u32,
                last_skipped: row.get::<i64, _>("last_skipped") as u32,
                last_failed: row.get::<i64, _>("last_failed") as u32,
            })
        })
        .transpose()
    }
}

/// SQLite-backed import ledger
#[derive(Clone)]
pub struct SqliteIngestLedger {
    pool: SqlitePool,
}

impl SqliteIngestLedger {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

async fn ensure_source_active(
    conn: &mut sqlx::SqliteConnection,
    vault: VaultId,
    source_id: Id,
) -> Result<()> {
    // Configured sources have no persisted row.
    let blocked: Option<bool> = sqlx::query_scalar(
        "SELECT deleted_at IS NOT NULL OR vault_id != ? FROM ingestion_sources WHERE id = ?",
    )
    .bind(vault.to_string())
    .bind(source_id.to_string())
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
impl IngestLedger for SqliteIngestLedger {
    async fn insert(&self, vault: VaultId, entry: &LedgerEntry) -> Result<bool> {
        if entry.vault_id != vault {
            return Err(Error::Validation(
                "Ingest entry belongs to a different vault".into(),
            ));
        }
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        ensure_source_active(&mut tx, vault, entry.source_id).await?;
        let result = sqlx::query(
            "INSERT INTO ingest_ledger (vault_id, key, source_id, document_id, outcome, created_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(vault_id, key) DO NOTHING",
        )
        .bind(vault.to_string())
        .bind(&entry.key)
        .bind(entry.source_id.to_string())
        .bind(entry.document_id.map(|id| id.to_string()))
        .bind(&entry.outcome)
        .bind(entry.created_at.to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        tx.commit().await.map_err(repo_err)?;
        Ok(result.rows_affected() == 1)
    }

    async fn find(&self, vault: VaultId, key: &str) -> Result<Option<LedgerEntry>> {
        let row = sqlx::query("SELECT * FROM ingest_ledger WHERE vault_id = ? AND key = ?")
            .bind(vault.to_string())
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.map(|row| {
            let document_id: Option<&str> = row.get("document_id");
            Ok(LedgerEntry {
                vault_id: VaultId(parse_id(row.get("vault_id"))?),
                key: row.get("key"),
                source_id: parse_id(row.get("source_id"))?,
                document_id: document_id.map(parse_id).transpose()?,
                outcome: row.get("outcome"),
                created_at: parse_time(row.get("created_at"))?,
            })
        })
        .transpose()
    }

    async fn complete(
        &self,
        vault: VaultId,
        key: &str,
        outcome: &str,
        document_id: Option<Id>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE ingest_ledger SET outcome = ?, document_id = ? WHERE vault_id = ? AND key = ?",
        )
        .bind(outcome)
        .bind(document_id.map(|id| id.to_string()))
        .bind(vault.to_string())
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
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(repo_err)?;
        if outcome == "claimed" {
            ensure_source_active(&mut tx, vault, expected.source_id).await?;
        }
        let result = sqlx::query(
            "UPDATE ingest_ledger SET outcome = ?, created_at = ?
             WHERE vault_id = ? AND key = ? AND source_id = ?
               AND document_id IS ? AND outcome = ? AND created_at = ?",
        )
        .bind(outcome)
        .bind(created_at.to_rfc3339())
        .bind(vault.to_string())
        .bind(&expected.key)
        .bind(expected.source_id.to_string())
        .bind(expected.document_id.map(|id| id.to_string()))
        .bind(&expected.outcome)
        .bind(expected.created_at.to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        tx.commit().await.map_err(repo_err)?;
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
             WHERE vault_id = ? AND key = ? AND source_id = ?
               AND document_id IS ? AND outcome = ? AND created_at = ?",
        )
        .bind(vault.to_string())
        .bind(&expected.key)
        .bind(expected.source_id.to_string())
        .bind(expected.document_id.map(|id| id.to_string()))
        .bind(&expected.outcome)
        .bind(expected.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() == 1)
    }

    async fn delete(&self, vault: VaultId, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM ingest_ledger WHERE vault_id = ? AND key = ?")
            .bind(vault.to_string())
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}
