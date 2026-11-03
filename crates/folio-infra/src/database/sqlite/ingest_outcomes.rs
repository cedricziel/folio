use chrono::{DateTime, Utc};
use folio_core::error::{Error, Result};
use folio_core::models::{
    Id, IngestAttempt, IngestClaim, IngestItemKey, IngestOutcomePage, IngestOutcomeRecord,
    IngestResult, LedgerEntry, SourceKind, SourceOrigin, SourceStatus, VaultId,
};
use sqlx::{Row, SqlitePool};

#[derive(Clone)]
pub struct SqliteIngestOutcomeRepository {
    pool: SqlitePool,
}

impl SqliteIngestOutcomeRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn record_source_status(
        &self,
        vault: VaultId,
        source: Id,
        origin: SourceOrigin,
        kind: SourceKind,
        status: &SourceStatus,
    ) -> Result<()> {
        if !matches!(kind, SourceKind::Folder | SourceKind::Email) {
            return Err(Error::Validation("Invalid source kind".into()));
        }
        let failed = status.last_error.is_some();
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let inserted = sqlx::query("INSERT INTO source_health (vault_id, source_id, source_managed, source_kind, failed) VALUES (?, ?, ?, ?, ?) ON CONFLICT(vault_id, source_id) DO NOTHING")
            .bind(vault.to_string()).bind(source.to_string()).bind(origin == SourceOrigin::Managed).bind(kind.as_str()).bind(failed)
            .execute(&mut *tx).await.map_err(repo_err)?.rows_affected() == 1;
        let row = sqlx::query(
            "SELECT source_managed, failed FROM source_health WHERE vault_id = ? AND source_id = ?",
        )
        .bind(vault.to_string())
        .bind(source.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_err)?;
        if row.get::<bool, _>("source_managed") != (origin == SourceOrigin::Managed)
            || !active_source(&mut tx, vault, source, kind, origin).await?
        {
            return Err(Error::Validation(
                "Source is no longer active in this vault".into(),
            ));
        }
        let change = folio_core::models::SourceHealthChange::between(
            if inserted {
                None
            } else {
                Some(row.get("failed"))
            },
            failed,
        );
        sqlx::query("UPDATE source_health SET failed = ?, source_kind = ? WHERE vault_id = ? AND source_id = ?")
            .bind(failed).bind(kind.as_str()).bind(vault.to_string()).bind(source.to_string())
            .execute(&mut *tx).await.map_err(repo_err)?;
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
        .bind(source.to_string())
        .bind(status.last_run_at.map(|t| t.to_rfc3339()))
        .bind(&status.last_error)
        .bind(status.last_imported)
        .bind(status.last_skipped)
        .bind(status.last_failed)
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;

        if let Some(change) = change {
            let event = super::super::ingest_outcomes::health_event(vault, source, kind, change)?;
            super::insert_domain_event(&mut tx, &event).await?;
        }
        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }

    pub async fn claim(
        &self,
        vault: VaultId,
        source_id: Id,
        source_kind: SourceKind,
        source_origin: SourceOrigin,
        key: &IngestItemKey,
        decision_fingerprint: &str,
    ) -> Result<IngestClaim> {
        super::super::ingest_outcomes::validate_observation(source_kind, decision_fingerprint)?;
        let now = Utc::now();
        let expires = now + chrono::Duration::minutes(10);
        let item_id = Id::new_v4();
        let attempt_id = Id::new_v4();
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        // The first write serializes SQLite claimers before they inspect the lease.
        let inserted = sqlx::query(
            "INSERT INTO ingest_items (id, vault_id, source_id, source_kind, source_managed, locator_key,
                attempt_id, started_at, lease_expires_at, decision_fingerprint, state)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'processing')
             ON CONFLICT(vault_id, source_id, locator_key) DO NOTHING",
        )
        .bind(item_id.to_string())
        .bind(vault.to_string())
        .bind(source_id.to_string())
        .bind(source_kind.as_str())
        .bind(source_origin == SourceOrigin::Managed)
        .bind(key.as_str())
        .bind(attempt_id.to_string())
        .bind(now.to_rfc3339())
        .bind(expires.to_rfc3339())
        .bind(decision_fingerprint)
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?
        .rows_affected()
            == 1;
        let row = sqlx::query("SELECT id, state, lease_expires_at, decision_fingerprint, source_managed FROM ingest_items WHERE vault_id = ? AND source_id = ? AND locator_key = ?")
            .bind(vault.to_string()).bind(source_id.to_string()).bind(key.as_str())
            .fetch_one(&mut *tx).await.map_err(repo_err)?;
        let id = Id::parse_str(row.get("id")).map_err(repo_err)?;
        if row.get::<bool, _>("source_managed") != (source_origin == SourceOrigin::Managed)
            || !active_source(&mut tx, vault, source_id, source_kind, source_origin).await?
        {
            tx.rollback().await.map_err(repo_err)?;
            return Ok(IngestClaim::Busy);
        }
        let state: &str = row.get("state");
        let lease = parse_time(row.get("lease_expires_at"))?;
        let fingerprint: &str = row.get("decision_fingerprint");
        let completed = super::super::ingest_outcomes::completed_observation(
            state,
            fingerprint,
            decision_fingerprint,
        )?;
        let claim = if inserted {
            IngestClaim::Claimed(IngestAttempt {
                item_id: id,
                attempt_id,
                vault_id: vault,
                source_id,
                source_kind,
                source_origin,
                started_at: now,
            })
        } else if let Some(outcome) = completed {
            IngestClaim::Completed(outcome)
        } else if state == "processing" && lease > now {
            IngestClaim::Busy
        } else {
            sqlx::query("UPDATE ingest_items SET attempt_id = ?, started_at = ?, lease_expires_at = ?, decision_fingerprint = ?, state = 'processing' WHERE id = ?")
                .bind(attempt_id.to_string()).bind(now.to_rfc3339()).bind(expires.to_rfc3339())
                .bind(decision_fingerprint).bind(id.to_string()).execute(&mut *tx).await.map_err(repo_err)?;
            IngestClaim::Claimed(IngestAttempt {
                item_id: id,
                attempt_id,
                vault_id: vault,
                source_id,
                source_kind,
                source_origin,
                started_at: now,
            })
        };
        tx.commit().await.map_err(repo_err)?;
        Ok(claim)
    }

    pub async fn complete(
        &self,
        attempt: &IngestAttempt,
        outcome: IngestResult,
        document_id: Option<Id>,
        content_claim: Option<&LedgerEntry>,
    ) -> Result<Option<IngestOutcomeRecord>> {
        super::super::ingest_outcomes::validate_completion(
            attempt,
            outcome,
            document_id,
            content_claim,
        )?;
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        sqlx::query(
            "UPDATE ingest_items SET state = state WHERE vault_id = ? AND source_id = ? AND id = ?",
        )
        .bind(attempt.vault_id.to_string())
        .bind(attempt.source_id.to_string())
        .bind(attempt.item_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        let context = sqlx::query("SELECT source_managed, source_kind FROM ingest_items WHERE vault_id = ? AND source_id = ? AND id = ?")
            .bind(attempt.vault_id.to_string()).bind(attempt.source_id.to_string()).bind(attempt.item_id.to_string())
            .fetch_optional(&mut *tx).await.map_err(repo_err)?;
        let valid_context = context.is_some_and(|row| {
            row.get::<bool, _>("source_managed") == (attempt.source_origin == SourceOrigin::Managed)
                && row.get::<&str, _>("source_kind") == attempt.source_kind.as_str()
        });
        if !valid_context
            || !active_source(
                &mut tx,
                attempt.vault_id,
                attempt.source_id,
                attempt.source_kind,
                attempt.source_origin,
            )
            .await?
        {
            tx.rollback().await.map_err(repo_err)?;
            return Ok(None);
        }
        let replay = sqlx::query("SELECT * FROM ingest_outcomes WHERE vault_id = ? AND source_id = ? AND item_id = ? AND attempt_id = ?")
            .bind(attempt.vault_id.to_string()).bind(attempt.source_id.to_string())
            .bind(attempt.item_id.to_string()).bind(attempt.attempt_id.to_string())
            .fetch_optional(&mut *tx).await.map_err(repo_err)?;
        if let Some(row) = replay {
            let record = decode(row, attempt.vault_id, attempt.source_id)?;
            if record.outcome != outcome || record.document_id != document_id {
                return Err(Error::Validation(
                    "Ingestion outcome replay changed its result".into(),
                ));
            }
            tx.commit().await.map_err(repo_err)?;
            return Ok(Some(record));
        }
        let changed = sqlx::query("UPDATE ingest_items SET state = ? WHERE vault_id = ? AND source_id = ? AND id = ? AND attempt_id = ? AND source_kind = ? AND state = 'processing'")
            .bind(outcome.as_str()).bind(attempt.vault_id.to_string()).bind(attempt.source_id.to_string())
            .bind(attempt.item_id.to_string()).bind(attempt.attempt_id.to_string()).bind(attempt.source_kind.as_str())
            .execute(&mut *tx).await.map_err(repo_err)?.rows_affected();
        if changed != 1 {
            tx.rollback().await.map_err(repo_err)?;
            return Ok(None);
        }
        if let Some(document) = document_id {
            let exists = sqlx::query("SELECT id FROM documents WHERE vault_id = ? AND id = ? AND deleted_at IS NULL AND file_path IS NOT NULL")
                .bind(attempt.vault_id.to_string()).bind(document.to_string())
                .fetch_optional(&mut *tx).await.map_err(repo_err)?.is_some();
            if !exists {
                tx.rollback().await.map_err(repo_err)?;
                return Ok(None);
            }
        }
        if let Some(claim) = content_claim {
            let terminal = if outcome == IngestResult::Imported {
                "imported"
            } else {
                "retryable"
            };
            let changed = sqlx::query("UPDATE ingest_ledger SET outcome = ? WHERE vault_id = ? AND key = ? AND source_id = ? AND document_id IS ? AND outcome = ? AND created_at = ?")
                .bind(terminal).bind(attempt.vault_id.to_string()).bind(&claim.key)
                .bind(claim.source_id.to_string()).bind(claim.document_id.map(|id| id.to_string()))
                .bind(&claim.outcome).bind(claim.created_at.to_rfc3339())
                .execute(&mut *tx).await.map_err(repo_err)?.rows_affected();
            if changed != 1 {
                tx.rollback().await.map_err(repo_err)?;
                return Ok(None);
            }
        }
        let event = super::super::ingest_outcomes::outcome_event(attempt, outcome)?;
        super::insert_domain_event(&mut tx, &event).await?;
        let id: i64 = sqlx::query_scalar("INSERT INTO ingest_outcomes (vault_id, source_id, item_id, attempt_id, event_id, occurred_at, outcome, document_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id")
            .bind(attempt.vault_id.to_string()).bind(attempt.source_id.to_string())
            .bind(attempt.item_id.to_string()).bind(attempt.attempt_id.to_string())
            .bind(event.event_id().to_string()).bind(event.occurred_at().to_rfc3339())
            .bind(outcome.as_str()).bind(document_id.map(|id| id.to_string()))
            .fetch_one(&mut *tx).await.map_err(repo_err)?;
        tx.commit().await.map_err(repo_err)?;
        Ok(Some(IngestOutcomeRecord {
            id,
            vault_id: attempt.vault_id,
            source_id: attempt.source_id,
            item_id: attempt.item_id,
            attempt_id: attempt.attempt_id,
            event_id: event.event_id(),
            occurred_at: event.occurred_at(),
            outcome,
            document_id,
        }))
    }

    pub async fn release(&self, attempt: &IngestAttempt) -> Result<bool> {
        Ok(sqlx::query("UPDATE ingest_items SET lease_expires_at = started_at WHERE vault_id = ? AND source_id = ? AND id = ? AND attempt_id = ? AND state = 'processing' AND lease_expires_at > started_at")
            .bind(attempt.vault_id.to_string()).bind(attempt.source_id.to_string())
            .bind(attempt.item_id.to_string()).bind(attempt.attempt_id.to_string())
            .execute(&self.pool).await.map_err(repo_err)?.rows_affected() == 1)
    }

    pub async fn page(
        &self,
        vault: VaultId,
        source_id: Id,
        before: Option<i64>,
        limit: u32,
    ) -> Result<IngestOutcomePage> {
        super::super::ingest_outcomes::validate_page(before, limit)?;
        let rows = sqlx::query("SELECT * FROM ingest_outcomes WHERE vault_id = ? AND source_id = ? AND (? IS NULL OR id < ?) ORDER BY id DESC LIMIT ?")
            .bind(vault.to_string()).bind(source_id.to_string()).bind(before).bind(before).bind(i64::from(limit) + 1)
            .fetch_all(&self.pool).await.map_err(repo_err)?;
        let mut outcomes = rows
            .into_iter()
            .map(|row| decode(row, vault, source_id))
            .collect::<Result<Vec<_>>>()?;
        let has_more = outcomes.len() > limit as usize;
        outcomes.truncate(limit as usize);
        let next_before = has_more.then(|| outcomes.last().unwrap().id);
        Ok(IngestOutcomePage {
            vault_id: vault,
            source_id,
            outcomes,
            next_before,
        })
    }
}

fn repo_err(error: impl std::fmt::Display) -> Error {
    Error::Repository(error.to_string())
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&Utc))
        .map_err(repo_err)
}

#[async_trait::async_trait]
impl folio_core::ports::IngestOutcomeReader for SqliteIngestOutcomeRepository {
    async fn page(
        &self,
        vault: VaultId,
        source_id: Id,
        before: Option<i64>,
        limit: u32,
    ) -> Result<IngestOutcomePage> {
        self.page(vault, source_id, before, limit).await
    }
}

fn decode(
    row: sqlx::sqlite::SqliteRow,
    vault: VaultId,
    source_id: Id,
) -> Result<IngestOutcomeRecord> {
    let document: Option<&str> = row.get("document_id");
    Ok(IngestOutcomeRecord {
        id: row.get("id"),
        vault_id: vault,
        source_id,
        item_id: Id::parse_str(row.get("item_id")).map_err(repo_err)?,
        attempt_id: Id::parse_str(row.get("attempt_id")).map_err(repo_err)?,
        event_id: Id::parse_str(row.get("event_id")).map_err(repo_err)?,
        occurred_at: parse_time(row.get("occurred_at"))?,
        outcome: super::super::ingest_outcomes::parse_result(row.get("outcome"))?,
        document_id: document.map(Id::parse_str).transpose().map_err(repo_err)?,
    })
}

#[async_trait::async_trait]
impl folio_core::ports::IngestOutcomeRepository for SqliteIngestOutcomeRepository {
    async fn record_source_status(
        &self,
        vault: VaultId,
        source: Id,
        origin: SourceOrigin,
        kind: SourceKind,
        status: &SourceStatus,
    ) -> Result<()> {
        self.record_source_status(vault, source, origin, kind, status)
            .await
    }

    async fn claim(
        &self,
        vault: VaultId,
        source_id: Id,
        source_kind: SourceKind,
        source_origin: SourceOrigin,
        key: &IngestItemKey,
        decision_fingerprint: &str,
    ) -> Result<IngestClaim> {
        self.claim(
            vault,
            source_id,
            source_kind,
            source_origin,
            key,
            decision_fingerprint,
        )
        .await
    }
    async fn complete(
        &self,
        attempt: &IngestAttempt,
        outcome: IngestResult,
        document_id: Option<Id>,
        content_claim: Option<&LedgerEntry>,
    ) -> Result<Option<IngestOutcomeRecord>> {
        self.complete(attempt, outcome, document_id, content_claim)
            .await
    }
    async fn release(&self, attempt: &IngestAttempt) -> Result<bool> {
        self.release(attempt).await
    }
}

pub(super) async fn active_source(
    conn: &mut sqlx::SqliteConnection,
    vault: VaultId,
    source: Id,
    kind: SourceKind,
    origin: SourceOrigin,
) -> Result<bool> {
    if origin == SourceOrigin::Config {
        return Ok(true);
    }
    Ok(sqlx::query("SELECT id FROM ingestion_sources WHERE vault_id = ? AND id = ? AND kind = ? AND enabled = 1")
        .bind(vault.to_string()).bind(source.to_string()).bind(kind.as_str()).fetch_optional(conn).await.map_err(repo_err)?.is_some())
}
