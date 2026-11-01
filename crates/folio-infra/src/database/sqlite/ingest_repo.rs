//! SQLite repositories for ingestion sources and the import ledger

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{sqlite::SqliteRow, Row, SqlitePool};

use folio_core::error::{Error, Result};
use folio_core::models::{
    Id, IngestionSourceDef, IngestionSourceInput, LedgerEntry, SourceOrigin, SourceStatus,
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
        id: parse_id(row.get("id"))?,
        name: row.get("name"),
        enabled: row.get::<i64, _>("enabled") != 0,
        origin: SourceOrigin::Managed,
        config: serde_json::from_str(row.get("config")).map_err(repo_err)?,
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
    async fn list(&self) -> Result<Vec<IngestionSourceDef>> {
        let rows = sqlx::query("SELECT * FROM ingestion_sources ORDER BY created_at, name")
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(source_from_row).collect()
    }

    async fn find(&self, id: Id) -> Result<Option<IngestionSourceDef>> {
        let row = sqlx::query("SELECT * FROM ingestion_sources WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(source_from_row).transpose()
    }

    async fn create(&self, input: IngestionSourceInput) -> Result<IngestionSourceDef> {
        let id = Id::new_v4();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO ingestion_sources (id, name, enabled, kind, config, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.config.kind().as_str())
        .bind(serde_json::to_string(&input.config).map_err(repo_err)?)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| write_err(e, &input.name))?;
        self.find(id)
            .await?
            .ok_or_else(|| repo_err("Source vanished after insert"))
    }

    async fn update(&self, id: Id, input: IngestionSourceInput) -> Result<IngestionSourceDef> {
        let result = sqlx::query(
            "UPDATE ingestion_sources
             SET name = ?, enabled = ?, kind = ?, config = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&input.name)
        .bind(input.enabled)
        .bind(input.config.kind().as_str())
        .bind(serde_json::to_string(&input.config).map_err(repo_err)?)
        .bind(Utc::now().to_rfc3339())
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| write_err(e, &input.name))?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound {
                entity_type: "Source",
                id: id.to_string(),
            });
        }
        self.find(id)
            .await?
            .ok_or_else(|| repo_err("Source vanished after update"))
    }

    async fn delete(&self, id: Id) -> Result<()> {
        let result = sqlx::query("DELETE FROM ingestion_sources WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound {
                entity_type: "Source",
                id: id.to_string(),
            });
        }
        sqlx::query("DELETE FROM ingestion_status WHERE source_id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
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

#[async_trait]
impl IngestLedger for SqliteIngestLedger {
    async fn insert(&self, entry: &LedgerEntry) -> Result<bool> {
        let result = sqlx::query(
            "INSERT INTO ingest_ledger (key, source_id, document_id, outcome, created_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(key) DO NOTHING",
        )
        .bind(&entry.key)
        .bind(entry.source_id.to_string())
        .bind(entry.document_id.map(|id| id.to_string()))
        .bind(&entry.outcome)
        .bind(entry.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(result.rows_affected() == 1)
    }

    async fn find(&self, key: &str) -> Result<Option<LedgerEntry>> {
        let row = sqlx::query("SELECT * FROM ingest_ledger WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.map(|row| {
            let document_id: Option<&str> = row.get("document_id");
            Ok(LedgerEntry {
                key: row.get("key"),
                source_id: parse_id(row.get("source_id"))?,
                document_id: document_id.map(parse_id).transpose()?,
                outcome: row.get("outcome"),
                created_at: parse_time(row.get("created_at"))?,
            })
        })
        .transpose()
    }

    async fn complete(&self, key: &str, outcome: &str, document_id: Option<Id>) -> Result<()> {
        sqlx::query("UPDATE ingest_ledger SET outcome = ?, document_id = ? WHERE key = ?")
            .bind(outcome)
            .bind(document_id.map(|id| id.to_string()))
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM ingest_ledger WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}
