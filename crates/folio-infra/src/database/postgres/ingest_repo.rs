//! PostgreSQL repositories for ingestion sources and the import ledger

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgRow, PgPool, Row};

use folio_core::error::{Error, Result};
use folio_core::models::{
    Id, IngestionSourceDef, IngestionSourceInput, LedgerEntry, SourceOrigin, SourceStatus, VaultId,
};
use folio_core::ports::{IngestLedger, SourceRepository};

fn repo_err(e: impl std::fmt::Display) -> Error {
    Error::Repository(e.to_string())
}

fn write_err(e: sqlx::Error, name: &str) -> Error {
    match e.as_database_error() {
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
        id: row.get("id"),
        name: row.get("name"),
        enabled: row.get("enabled"),
        origin: SourceOrigin::Managed,
        config: serde_json::from_str(row.get("config")).map_err(repo_err)?,
        vault_id: Some(VaultId(row.get("vault_id"))),
    })
}

#[async_trait]
impl SourceRepository for PostgresSourceRepository {
    async fn list(&self, vault: VaultId) -> Result<Vec<IngestionSourceDef>> {
        let rows = sqlx::query(
            "SELECT * FROM ingestion_sources WHERE vault_id = $1 ORDER BY created_at, name",
        )
        .bind(vault.0)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(source_from_row).collect()
    }

    async fn list_all(&self) -> Result<Vec<IngestionSourceDef>> {
        let rows = sqlx::query("SELECT * FROM ingestion_sources ORDER BY created_at, name")
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(source_from_row).collect()
    }

    async fn find(&self, vault: VaultId, id: Id) -> Result<Option<IngestionSourceDef>> {
        let row = sqlx::query("SELECT * FROM ingestion_sources WHERE id = $1 AND vault_id = $2")
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
        let id = Id::new_v4();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO ingestion_sources
                 (id, vault_id, name, enabled, kind, config, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
        )
        .bind(id)
        .bind(vault.0)
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.config.kind().as_str())
        .bind(serde_json::to_string(&input.config).map_err(repo_err)?)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| write_err(e, &input.name))?;
        self.find(vault, id)
            .await?
            .ok_or_else(|| repo_err("Source vanished after insert"))
    }

    async fn update(
        &self,
        vault: VaultId,
        id: Id,
        input: IngestionSourceInput,
    ) -> Result<IngestionSourceDef> {
        let result = sqlx::query(
            "UPDATE ingestion_sources
             SET name = $1, enabled = $2, kind = $3, config = $4, updated_at = $5
             WHERE id = $6 AND vault_id = $7",
        )
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.config.kind().as_str())
        .bind(serde_json::to_string(&input.config).map_err(repo_err)?)
        .bind(Utc::now())
        .bind(id)
        .bind(vault.0)
        .execute(&self.pool)
        .await
        .map_err(|e| write_err(e, &input.name))?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound {
                entity_type: "Source",
                id: id.to_string(),
            });
        }
        self.find(vault, id)
            .await?
            .ok_or_else(|| repo_err("Source vanished after update"))
    }

    async fn delete(&self, vault: VaultId, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM ingestion_sources WHERE id = $1 AND vault_id = $2")
            .bind(id)
            .bind(vault.0)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound {
                entity_type: "Source",
                id: id.to_string(),
            });
        }
        sqlx::query("DELETE FROM ingestion_status WHERE source_id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
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

#[async_trait]
impl IngestLedger for PostgresIngestLedger {
    async fn insert(&self, vault: VaultId, entry: &LedgerEntry) -> Result<bool> {
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
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
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
