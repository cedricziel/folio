use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use folio_core::error::{Error, Result};
use folio_core::models::{
    CreateImportRun, Id, ImportApplied, ImportItem, ImportItemCount, ImportItemOutcome,
    ImportItemStatus, ImportRun, ImportRunStatus, NewImportItem, NewImportRun,
    PaperlessLedgerEntry, PaperlessSyncState, TaxonomyKind, TaxonomyMapping, UserId, VaultId,
    IMPORT_CLAIM_TIMEOUT,
};
use folio_core::ports::{ImportItemRepository, ImportRunRepository, PaperlessLedgerRepository};
use sqlx::{PgConnection, PgPool};

/// Runs, items, the sync ledger and the sync state in PostgreSQL
pub struct PostgresImportRepository {
    pool: PgPool,
}

impl PostgresImportRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn storage_error(error: impl std::fmt::Display) -> Error {
    Error::Repository(error.to_string())
}

fn parse<T: FromStr<Err = String>>(text: &str) -> Result<T> {
    text.parse().map_err(Error::Repository)
}

fn json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(storage_error)
}

fn to_json(value: &impl serde::Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(storage_error)
}

fn unsigned(value: i32) -> Result<u32> {
    u32::try_from(value).map_err(storage_error)
}

const RUN_COLUMNS: &str = r#"id, vault_id, kind, "trigger", source_id, source_path, dry_run, status, cursor_after, confirm_trash, plan_complete, cancel_requested, driver_restarts, params::text AS params, summary::text AS summary, error, created_by, created_at, started_at, finished_at"#;

#[derive(sqlx::FromRow)]
struct RunRow {
    id: Id,
    vault_id: Id,
    kind: String,
    trigger: String,
    source_id: Option<Id>,
    source_path: Option<String>,
    dry_run: bool,
    status: String,
    cursor_after: Option<DateTime<Utc>>,
    confirm_trash: bool,
    plan_complete: bool,
    cancel_requested: bool,
    driver_restarts: i32,
    params: String,
    summary: Option<String>,
    error: Option<String>,
    created_by: Option<String>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
}

impl TryFrom<RunRow> for ImportRun {
    type Error = Error;

    fn try_from(row: RunRow) -> Result<Self> {
        Ok(Self {
            id: row.id,
            vault_id: VaultId(row.vault_id),
            kind: parse(&row.kind)?,
            trigger: parse(&row.trigger)?,
            source_id: row.source_id,
            source_path: row.source_path,
            dry_run: row.dry_run,
            status: parse(&row.status)?,
            cursor_after: row.cursor_after,
            confirm_trash: row.confirm_trash,
            plan_complete: row.plan_complete,
            cancel_requested: row.cancel_requested,
            driver_restarts: unsigned(row.driver_restarts)?,
            params: json(&row.params)?,
            summary: row.summary.as_deref().map(json).transpose()?,
            error: row.error,
            created_by: row.created_by.map(UserId),
            created_at: row.created_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
        })
    }
}

const ITEM_COLUMNS: &str = "run_id, paperless_id, action, document_id, title, status, reason, warnings::text AS warnings, payload::text AS payload, attempts, claim, claimed_at, updated_at";

#[derive(sqlx::FromRow)]
struct ItemRow {
    run_id: Id,
    paperless_id: i64,
    action: String,
    document_id: Option<Id>,
    title: Option<String>,
    status: String,
    reason: Option<String>,
    warnings: String,
    payload: Option<String>,
    attempts: i32,
    claim: Option<Id>,
    claimed_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<ItemRow> for ImportItem {
    type Error = Error;

    fn try_from(row: ItemRow) -> Result<Self> {
        Ok(Self {
            run_id: row.run_id,
            paperless_id: row.paperless_id,
            action: parse(&row.action)?,
            document_id: row.document_id,
            title: row.title,
            status: parse(&row.status)?,
            reason: row.reason,
            warnings: json(&row.warnings)?,
            payload: row.payload.as_deref().map(json).transpose()?,
            attempts: unsigned(row.attempts)?,
            claim: row.claim,
            claimed_at: row.claimed_at,
            updated_at: row.updated_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct LedgerRow {
    vault_id: Id,
    paperless_id: i64,
    document_id: Id,
    paperless_modified: DateTime<Utc>,
    checksum: String,
    fingerprint: String,
    needs_reapply: bool,
    trashed_in_paperless: bool,
    folio_state: String,
    last_synced_at: DateTime<Utc>,
    last_run_id: Option<Id>,
}

impl TryFrom<LedgerRow> for PaperlessLedgerEntry {
    type Error = Error;

    fn try_from(row: LedgerRow) -> Result<Self> {
        Ok(Self {
            vault_id: VaultId(row.vault_id),
            paperless_id: row.paperless_id,
            document_id: row.document_id,
            paperless_modified: row.paperless_modified,
            checksum: row.checksum,
            fingerprint: row.fingerprint,
            needs_reapply: row.needs_reapply,
            trashed_in_paperless: row.trashed_in_paperless,
            folio_state: parse(&row.folio_state)?,
            last_synced_at: row.last_synced_at,
            last_run_id: row.last_run_id,
        })
    }
}

const TAXONOMY_COLUMNS: &str = "vault_id, kind, paperless_id, entity_id, name";

#[derive(sqlx::FromRow)]
struct TaxonomyRow {
    vault_id: Id,
    kind: String,
    paperless_id: i64,
    entity_id: Id,
    name: String,
}

impl TryFrom<TaxonomyRow> for TaxonomyMapping {
    type Error = Error;

    fn try_from(row: TaxonomyRow) -> Result<Self> {
        Ok(Self {
            vault_id: VaultId(row.vault_id),
            kind: parse(&row.kind)?,
            paperless_id: row.paperless_id,
            entity_id: row.entity_id,
            name: row.name,
        })
    }
}

#[derive(sqlx::FromRow)]
struct SyncStateRow {
    vault_id: Id,
    source_id: Option<Id>,
    library_sample: Option<String>,
    cursor_modified: Option<DateTime<Utc>>,
    last_poll_at: Option<DateTime<Utc>>,
    last_full_reconcile_at: Option<DateTime<Utc>>,
    once_done: bool,
    retry_ids: String,
    parked_ids: String,
    updated_at: DateTime<Utc>,
}

impl TryFrom<SyncStateRow> for PaperlessSyncState {
    type Error = Error;

    fn try_from(row: SyncStateRow) -> Result<Self> {
        Ok(Self {
            vault_id: VaultId(row.vault_id),
            source_id: row.source_id,
            library_sample: row.library_sample.as_deref().map(json).transpose()?,
            cursor_modified: row.cursor_modified,
            last_poll_at: row.last_poll_at,
            last_full_reconcile_at: row.last_full_reconcile_at,
            once_done: row.once_done,
            retry_ids: json(&row.retry_ids)?,
            parked_ids: json(&row.parked_ids)?,
            updated_at: row.updated_at,
        })
    }
}

fn missing_run(id: Id) -> Error {
    Error::NotFound {
        entity_type: "Import run",
        id: id.to_string(),
    }
}

/// Write a ledger row inside the caller's transaction, so it commits or rolls
/// back with the document change it records
pub async fn upsert_paperless_document(
    conn: &mut PgConnection,
    entry: &PaperlessLedgerEntry,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO paperless_documents (vault_id, paperless_id, document_id, paperless_modified, checksum, fingerprint, needs_reapply, trashed_in_paperless, folio_state, last_synced_at, last_run_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
         ON CONFLICT (vault_id, paperless_id) DO UPDATE SET document_id = excluded.document_id, paperless_modified = excluded.paperless_modified, checksum = excluded.checksum, fingerprint = excluded.fingerprint, needs_reapply = excluded.needs_reapply, trashed_in_paperless = excluded.trashed_in_paperless, folio_state = excluded.folio_state, last_synced_at = excluded.last_synced_at, last_run_id = excluded.last_run_id",
    )
    .bind(entry.vault_id.0)
    .bind(entry.paperless_id)
    .bind(entry.document_id)
    .bind(entry.paperless_modified)
    .bind(&entry.checksum)
    .bind(&entry.fingerprint)
    .bind(entry.needs_reapply)
    .bind(entry.trashed_in_paperless)
    .bind(entry.folio_state.as_str())
    .bind(entry.last_synced_at)
    .bind(entry.last_run_id)
    .execute(conn)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// Write the ledger row and finish the claimed item inside the caller's
/// transaction; a lost claim is a `Conflict`, so the caller's `?` rolls the
/// document change back
pub async fn record_import_applied(
    conn: &mut PgConnection,
    applied: &ImportApplied,
    now: DateTime<Utc>,
) -> Result<()> {
    upsert_paperless_document(conn, &applied.ledger).await?;
    let paperless_id = applied.ledger.paperless_id;
    if finish_import_item(
        conn,
        applied.run_id,
        paperless_id,
        applied.claim,
        &applied.outcome,
        now,
    )
    .await?
    {
        Ok(())
    } else {
        Err(Error::Conflict(format!(
            "import item {paperless_id} is no longer held by this claim"
        )))
    }
}

/// Returns whether `claim` still held the item
async fn finish_import_item(
    conn: &mut PgConnection,
    run: Id,
    paperless_id: i64,
    claim: Id,
    outcome: &ImportItemOutcome,
    now: DateTime<Utc>,
) -> Result<bool> {
    if matches!(
        outcome.status,
        ImportItemStatus::Pending | ImportItemStatus::InProgress
    ) {
        return Err(Error::Validation(format!(
            "an item can't finish as {}",
            outcome.status
        )));
    }
    let result = sqlx::query(
        "UPDATE import_items SET status = $1, reason = $2, warnings = $3::jsonb, updated_at = $4 \
         WHERE run_id = $5 AND paperless_id = $6 AND claim = $7 AND status = 'in_progress'",
    )
    .bind(outcome.status.as_str())
    .bind(&outcome.reason)
    .bind(to_json(&outcome.warnings)?)
    .bind(now)
    .bind(run)
    .bind(paperless_id)
    .bind(claim)
    .execute(conn)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() == 1)
}

#[async_trait]
impl ImportRunRepository for PostgresImportRepository {
    async fn create(&self, run: NewImportRun) -> Result<CreateImportRun> {
        // The active run can finish between a rejected insert and the lookup;
        // one more insert covers that.
        for _ in 0..2 {
            let inserted = sqlx::query_as::<_, RunRow>(sqlx::AssertSqlSafe(format!(
                r#"INSERT INTO import_runs (id, vault_id, kind, "trigger", source_id, source_path, dry_run, status, params, created_by, created_at)
                   VALUES ($1, $2, $3, $4, $5, $6, $7, 'queued', $8::jsonb, $9, $10) RETURNING {RUN_COLUMNS}"#
            )))
            .bind(Id::new_v4())
            .bind(run.vault_id.0)
            .bind(run.kind.as_str())
            .bind(run.trigger.as_str())
            .bind(run.source_id)
            .bind(&run.source_path)
            .bind(run.dry_run)
            .bind(to_json(&run.params)?)
            .bind(run.created_by.as_ref().map(UserId::as_str))
            .bind(Utc::now())
            .fetch_one(&self.pool)
            .await;
            match inserted {
                Ok(row) => return Ok(CreateImportRun::Created(Box::new(row.try_into()?))),
                Err(error)
                    if error
                        .as_database_error()
                        .is_some_and(|e| e.is_unique_violation()) =>
                {
                    let active: Option<Id> = sqlx::query_scalar(
                        "SELECT id FROM import_runs WHERE vault_id = $1 AND status IN ('queued', 'running')",
                    )
                    .bind(run.vault_id.0)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(storage_error)?;
                    if let Some(active_run) = active {
                        return Ok(CreateImportRun::Conflict { active_run });
                    }
                }
                Err(error) => return Err(storage_error(error)),
            }
        }
        Err(Error::Conflict(
            "import runs in this vault keep changing; try again".into(),
        ))
    }

    async fn get(&self, vault: VaultId, id: Id) -> Result<Option<ImportRun>> {
        sqlx::query_as::<_, RunRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {RUN_COLUMNS} FROM import_runs WHERE vault_id = $1 AND id = $2"
        )))
        .bind(vault.0)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn list(&self, vault: VaultId, limit: u32) -> Result<Vec<ImportRun>> {
        sqlx::query_as::<_, RunRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {RUN_COLUMNS} FROM import_runs WHERE vault_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2"
        )))
        .bind(vault.0)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn transition(
        &self,
        id: Id,
        from: &[ImportRunStatus],
        to: ImportRunStatus,
        error: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let from: Vec<&str> = from.iter().map(|status| status.as_str()).collect();
        let finished = (!to.is_active()).then_some(now);
        let result = sqlx::query(
            "UPDATE import_runs SET status = $1, \
                 started_at = CASE WHEN $1 = 'running' THEN $2 ELSE started_at END, \
                 finished_at = $3, error = $4 \
             WHERE id = $5 AND status = ANY($6)",
        )
        .bind(to.as_str())
        .bind(now)
        .bind(finished)
        .bind(error)
        .bind(id)
        .bind(from)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn set_plan_complete(&self, id: Id, complete: bool) -> Result<()> {
        self.update_run(
            sqlx::query("UPDATE import_runs SET plan_complete = $1 WHERE id = $2")
                .bind(complete)
                .bind(id),
            id,
        )
        .await
    }

    async fn request_cancel(&self, id: Id) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE import_runs SET cancel_requested = TRUE WHERE id = $1 AND status IN ('queued', 'running')",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn increment_driver_restarts(&self, id: Id) -> Result<u32> {
        let restarts: Option<i32> = sqlx::query_scalar(
            "UPDATE import_runs SET driver_restarts = driver_restarts + 1 WHERE id = $1 RETURNING driver_restarts",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        unsigned(restarts.ok_or_else(|| missing_run(id))?)
    }

    async fn set_cursor_after(&self, id: Id, cursor: Option<DateTime<Utc>>) -> Result<()> {
        self.update_run(
            sqlx::query("UPDATE import_runs SET cursor_after = $1 WHERE id = $2")
                .bind(cursor)
                .bind(id),
            id,
        )
        .await
    }

    async fn set_confirm_trash(&self, id: Id, confirm: bool) -> Result<()> {
        self.update_run(
            sqlx::query("UPDATE import_runs SET confirm_trash = $1 WHERE id = $2")
                .bind(confirm)
                .bind(id),
            id,
        )
        .await
    }

    async fn set_summary(&self, id: Id, summary: serde_json::Value) -> Result<()> {
        self.update_run(
            sqlx::query("UPDATE import_runs SET summary = $1::jsonb WHERE id = $2")
                .bind(to_json(&summary)?)
                .bind(id),
            id,
        )
        .await
    }
}

impl PostgresImportRepository {
    async fn update_run(
        &self,
        query: sqlx::query::Query<'_, sqlx::Postgres, sqlx::postgres::PgArguments>,
        id: Id,
    ) -> Result<()> {
        let result = query.execute(&self.pool).await.map_err(storage_error)?;
        if result.rows_affected() == 0 {
            return Err(missing_run(id));
        }
        Ok(())
    }
}

#[async_trait]
impl ImportItemRepository for PostgresImportRepository {
    async fn insert_items_ignore_existing(
        &self,
        run: Id,
        items: &[NewImportItem],
        now: DateTime<Utc>,
    ) -> Result<u64> {
        let payloads = items
            .iter()
            .map(|item| item.payload.as_ref().map(to_json).transpose())
            .collect::<Result<Vec<_>>>()?;
        let result = sqlx::query(
            "INSERT INTO import_items (run_id, paperless_id, action, document_id, title, status, reason, payload, updated_at) \
             SELECT $1, item.paperless_id, item.action, item.document_id, item.title, item.status, item.reason, item.payload::jsonb, $2 \
               FROM UNNEST($3::bigint[], $4::text[], $5::uuid[], $6::text[], $7::text[], $8::text[], $9::text[]) \
                    AS item(paperless_id, action, document_id, title, status, reason, payload) \
             ON CONFLICT (run_id, paperless_id) DO NOTHING",
        )
        .bind(run)
        .bind(now)
        .bind(items.iter().map(|item| item.paperless_id).collect::<Vec<_>>())
        .bind(items.iter().map(|item| item.action.as_str()).collect::<Vec<_>>())
        .bind(items.iter().map(|item| item.document_id).collect::<Vec<_>>())
        .bind(items.iter().map(|item| item.title.clone()).collect::<Vec<_>>())
        .bind(items.iter().map(|item| item.status.as_str()).collect::<Vec<_>>())
        .bind(items.iter().map(|item| item.reason.clone()).collect::<Vec<_>>())
        .bind(payloads)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(result.rows_affected())
    }

    async fn claim_next(
        &self,
        run: Id,
        claim: Id,
        now: DateTime<Utc>,
    ) -> Result<Option<ImportItem>> {
        // SKIP LOCKED lets concurrent claimers pass over each other's rows
        // instead of queueing behind them.
        sqlx::query_as::<_, ItemRow>(sqlx::AssertSqlSafe(format!(
            "UPDATE import_items SET status = 'in_progress', claim = $1, claimed_at = $2, attempts = attempts + 1, updated_at = $2 \
             WHERE run_id = $3 AND paperless_id = ( \
                 SELECT paperless_id FROM import_items \
                  WHERE run_id = $3 AND status IN ('pending', 'in_progress') \
                    AND (status = 'pending' OR claimed_at < $4) \
                  ORDER BY paperless_id LIMIT 1 FOR UPDATE SKIP LOCKED) \
             RETURNING {ITEM_COLUMNS}"
        )))
        .bind(claim)
        .bind(now)
        .bind(run)
        .bind(now - IMPORT_CLAIM_TIMEOUT)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn finish_item(
        &self,
        run: Id,
        paperless_id: i64,
        claim: Id,
        outcome: ImportItemOutcome,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        let mut conn = self.pool.acquire().await.map_err(storage_error)?;
        finish_import_item(&mut conn, run, paperless_id, claim, &outcome, now).await
    }

    async fn reset_failed_items(&self, run: Id, now: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE import_items SET status = 'pending', attempts = 0, claim = NULL, claimed_at = NULL, reason = NULL, warnings = '[]', updated_at = $1 \
             WHERE run_id = $2 AND status = 'failed'",
        )
        .bind(now)
        .bind(run)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(result.rows_affected())
    }

    async fn counts(&self, run: Id) -> Result<Vec<ImportItemCount>> {
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT action, status, COUNT(*) FROM import_items WHERE run_id = $1 GROUP BY action, status ORDER BY action, status",
        )
        .bind(run)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;
        rows.into_iter()
            .map(|(action, status, count)| {
                Ok(ImportItemCount {
                    action: parse(&action)?,
                    status: parse(&status)?,
                    count: u64::try_from(count).map_err(storage_error)?,
                })
            })
            .collect()
    }

    async fn list_items(
        &self,
        run: Id,
        status: Option<ImportItemStatus>,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<ImportItem>> {
        // A separate statement per filter lets the status index serve it.
        let filter = if status.is_some() {
            "AND status = $4"
        } else {
            ""
        };
        let mut query = sqlx::query_as::<_, ItemRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {ITEM_COLUMNS} FROM import_items \
             WHERE run_id = $1 AND paperless_id > $2 {filter} \
             ORDER BY paperless_id LIMIT $3"
        )))
        .bind(run)
        .bind(after.unwrap_or(i64::MIN))
        .bind(i64::from(limit));
        if let Some(status) = status {
            query = query.bind(status.as_str());
        }
        query
            .fetch_all(&self.pool)
            .await
            .map_err(storage_error)?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }
}

const LEDGER_COLUMNS: &str = "vault_id, paperless_id, document_id, paperless_modified, checksum, fingerprint, needs_reapply, trashed_in_paperless, folio_state, last_synced_at, last_run_id";

#[async_trait]
impl PaperlessLedgerRepository for PostgresImportRepository {
    async fn get_document(
        &self,
        vault: VaultId,
        paperless_id: i64,
    ) -> Result<Option<PaperlessLedgerEntry>> {
        sqlx::query_as::<_, LedgerRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {LEDGER_COLUMNS} FROM paperless_documents WHERE vault_id = $1 AND paperless_id = $2"
        )))
        .bind(vault.0)
        .bind(paperless_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn list_documents(
        &self,
        vault: VaultId,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<PaperlessLedgerEntry>> {
        sqlx::query_as::<_, LedgerRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {LEDGER_COLUMNS} FROM paperless_documents WHERE vault_id = $1 AND paperless_id > $2 ORDER BY paperless_id LIMIT $3"
        )))
        .bind(vault.0)
        .bind(after.unwrap_or(i64::MIN))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn upsert_document(&self, entry: &PaperlessLedgerEntry) -> Result<()> {
        let mut conn = self.pool.acquire().await.map_err(storage_error)?;
        upsert_paperless_document(&mut conn, entry).await
    }

    async fn get_taxonomy(
        &self,
        vault: VaultId,
        kind: TaxonomyKind,
        paperless_id: i64,
    ) -> Result<Option<TaxonomyMapping>> {
        sqlx::query_as::<_, TaxonomyRow>(
            sqlx::AssertSqlSafe(format!("SELECT {TAXONOMY_COLUMNS} FROM paperless_taxonomy WHERE vault_id = $1 AND kind = $2 AND paperless_id = $3")),
        )
        .bind(vault.0)
        .bind(kind.as_str())
        .bind(paperless_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn taxonomy_map(
        &self,
        vault: VaultId,
        kind: TaxonomyKind,
    ) -> Result<Vec<TaxonomyMapping>> {
        sqlx::query_as::<_, TaxonomyRow>(
            sqlx::AssertSqlSafe(format!("SELECT {TAXONOMY_COLUMNS} FROM paperless_taxonomy WHERE vault_id = $1 AND kind = $2 ORDER BY paperless_id")),
        )
        .bind(vault.0)
        .bind(kind.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn upsert_taxonomy(&self, mapping: &TaxonomyMapping) -> Result<()> {
        sqlx::query(
            "INSERT INTO paperless_taxonomy (vault_id, kind, paperless_id, entity_id, name) VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (vault_id, kind, paperless_id) DO UPDATE SET entity_id = excluded.entity_id, name = excluded.name",
        )
        .bind(mapping.vault_id.0)
        .bind(mapping.kind.as_str())
        .bind(mapping.paperless_id)
        .bind(mapping.entity_id)
        .bind(&mapping.name)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    async fn sync_state(&self, vault: VaultId) -> Result<Option<PaperlessSyncState>> {
        sqlx::query_as::<_, SyncStateRow>(
            "SELECT vault_id, source_id, library_sample::text AS library_sample, cursor_modified, last_poll_at, last_full_reconcile_at, once_done, retry_ids::text AS retry_ids, parked_ids::text AS parked_ids, updated_at \
             FROM paperless_sync_state WHERE vault_id = $1",
        )
        .bind(vault.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn put_sync_state(&self, state: &PaperlessSyncState) -> Result<()> {
        sqlx::query(
            "INSERT INTO paperless_sync_state (vault_id, source_id, library_sample, cursor_modified, last_poll_at, last_full_reconcile_at, once_done, retry_ids, parked_ids, updated_at) \
             VALUES ($1, $2, $3::jsonb, $4, $5, $6, $7, $8::jsonb, $9::jsonb, $10) \
             ON CONFLICT (vault_id) DO UPDATE SET source_id = excluded.source_id, library_sample = excluded.library_sample, cursor_modified = excluded.cursor_modified, last_poll_at = excluded.last_poll_at, last_full_reconcile_at = excluded.last_full_reconcile_at, once_done = excluded.once_done, retry_ids = excluded.retry_ids, parked_ids = excluded.parked_ids, updated_at = excluded.updated_at",
        )
        .bind(state.vault_id.0)
        .bind(state.source_id)
        .bind(state.library_sample.as_ref().map(to_json).transpose()?)
        .bind(state.cursor_modified)
        .bind(state.last_poll_at)
        .bind(state.last_full_reconcile_at)
        .bind(state.once_done)
        .bind(to_json(&state.retry_ids)?)
        .bind(to_json(&state.parked_ids)?)
        .bind(state.updated_at)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }
}
