use std::str::FromStr;

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use folio_core::error::{Error, Result};
use folio_core::models::{
    CreateImportRun, Id, ImportApplied, ImportItem, ImportItemCount, ImportItemOutcome,
    ImportItemStatus, ImportRun, ImportRunStatus, NewImportItem, NewImportRun,
    PaperlessLedgerEntry, PaperlessSyncState, RunTransition, TaxonomyKind, TaxonomyMapping, UserId,
    VaultId, IMPORT_CLAIM_TIMEOUT,
};
use folio_core::ports::{ImportItemRepository, ImportRunRepository, PaperlessLedgerRepository};
use sqlx::{Connection, SqliteConnection, SqlitePool};

/// Runs, items, the sync ledger and the sync state in SQLite
pub struct SqliteImportRepository {
    pool: SqlitePool,
}

impl SqliteImportRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn storage_error(error: impl std::fmt::Display) -> Error {
    Error::Repository(error.to_string())
}

/// Fixed fraction digits keep text comparison in time order
fn ts(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parse_ts(text: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .map(|time| time.with_timezone(&Utc))
        .map_err(storage_error)
}

fn parse_opt_ts(text: Option<String>) -> Result<Option<DateTime<Utc>>> {
    text.as_deref().map(parse_ts).transpose()
}

fn parse_id(text: &str) -> Result<Id> {
    Id::parse_str(text).map_err(storage_error)
}

fn parse_opt_id(text: Option<String>) -> Result<Option<Id>> {
    text.as_deref().map(parse_id).transpose()
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

const RUN_COLUMNS: &str = r#"id, vault_id, kind, "trigger", source_id, source_path, dry_run, status, cursor_after, confirm_trash, plan_complete, cancel_requested, driver_restarts, params, summary, error, created_by, created_at, started_at, finished_at"#;

#[derive(sqlx::FromRow)]
struct RunRow {
    id: String,
    vault_id: String,
    kind: String,
    trigger: String,
    source_id: Option<String>,
    source_path: Option<String>,
    dry_run: bool,
    status: String,
    cursor_after: Option<String>,
    confirm_trash: bool,
    plan_complete: bool,
    cancel_requested: bool,
    driver_restarts: i64,
    params: String,
    summary: Option<String>,
    error: Option<String>,
    created_by: Option<String>,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
}

impl TryFrom<RunRow> for ImportRun {
    type Error = Error;

    fn try_from(row: RunRow) -> Result<Self> {
        Ok(Self {
            id: parse_id(&row.id)?,
            vault_id: VaultId(parse_id(&row.vault_id)?),
            kind: parse(&row.kind)?,
            trigger: parse(&row.trigger)?,
            source_id: parse_opt_id(row.source_id)?,
            source_path: row.source_path,
            dry_run: row.dry_run,
            status: parse(&row.status)?,
            cursor_after: parse_opt_ts(row.cursor_after)?,
            confirm_trash: row.confirm_trash,
            plan_complete: row.plan_complete,
            cancel_requested: row.cancel_requested,
            driver_restarts: u32::try_from(row.driver_restarts).map_err(storage_error)?,
            params: json(&row.params)?,
            summary: row.summary.as_deref().map(json).transpose()?,
            error: row.error,
            created_by: row.created_by.map(UserId),
            created_at: parse_ts(&row.created_at)?,
            started_at: parse_opt_ts(row.started_at)?,
            finished_at: parse_opt_ts(row.finished_at)?,
        })
    }
}

const ITEM_COLUMNS: &str = "run_id, paperless_id, action, document_id, title, status, reason, warnings, payload, attempts, claim, claimed_at, updated_at";

#[derive(sqlx::FromRow)]
struct ItemRow {
    run_id: String,
    paperless_id: i64,
    action: String,
    document_id: Option<String>,
    title: Option<String>,
    status: String,
    reason: Option<String>,
    warnings: String,
    payload: Option<String>,
    attempts: i64,
    claim: Option<String>,
    claimed_at: Option<String>,
    updated_at: String,
}

impl TryFrom<ItemRow> for ImportItem {
    type Error = Error;

    fn try_from(row: ItemRow) -> Result<Self> {
        Ok(Self {
            run_id: parse_id(&row.run_id)?,
            paperless_id: row.paperless_id,
            action: parse(&row.action)?,
            document_id: parse_opt_id(row.document_id)?,
            title: row.title,
            status: parse(&row.status)?,
            reason: row.reason,
            warnings: json(&row.warnings)?,
            payload: row.payload.as_deref().map(json).transpose()?,
            attempts: u32::try_from(row.attempts).map_err(storage_error)?,
            claim: parse_opt_id(row.claim)?,
            claimed_at: parse_opt_ts(row.claimed_at)?,
            updated_at: parse_ts(&row.updated_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
struct LedgerRow {
    vault_id: String,
    paperless_id: i64,
    document_id: String,
    paperless_modified: String,
    checksum: String,
    fingerprint: String,
    needs_reapply: bool,
    trashed_in_paperless: bool,
    folio_state: String,
    last_synced_at: String,
    last_run_id: Option<String>,
}

impl TryFrom<LedgerRow> for PaperlessLedgerEntry {
    type Error = Error;

    fn try_from(row: LedgerRow) -> Result<Self> {
        Ok(Self {
            vault_id: VaultId(parse_id(&row.vault_id)?),
            paperless_id: row.paperless_id,
            document_id: parse_id(&row.document_id)?,
            paperless_modified: parse_ts(&row.paperless_modified)?,
            checksum: row.checksum,
            fingerprint: row.fingerprint,
            needs_reapply: row.needs_reapply,
            trashed_in_paperless: row.trashed_in_paperless,
            folio_state: parse(&row.folio_state)?,
            last_synced_at: parse_ts(&row.last_synced_at)?,
            last_run_id: parse_opt_id(row.last_run_id)?,
        })
    }
}

const TAXONOMY_COLUMNS: &str = "vault_id, kind, paperless_id, entity_id, name";

#[derive(sqlx::FromRow)]
struct TaxonomyRow {
    vault_id: String,
    kind: String,
    paperless_id: i64,
    entity_id: String,
    name: String,
}

impl TryFrom<TaxonomyRow> for TaxonomyMapping {
    type Error = Error;

    fn try_from(row: TaxonomyRow) -> Result<Self> {
        Ok(Self {
            vault_id: VaultId(parse_id(&row.vault_id)?),
            kind: parse(&row.kind)?,
            paperless_id: row.paperless_id,
            entity_id: parse_id(&row.entity_id)?,
            name: row.name,
        })
    }
}

#[derive(sqlx::FromRow)]
struct SyncStateRow {
    vault_id: String,
    source_id: Option<String>,
    library_sample: Option<String>,
    cursor_modified: Option<String>,
    last_poll_at: Option<String>,
    last_full_reconcile_at: Option<String>,
    once_done: bool,
    retry_ids: String,
    parked_ids: String,
    updated_at: String,
}

impl TryFrom<SyncStateRow> for PaperlessSyncState {
    type Error = Error;

    fn try_from(row: SyncStateRow) -> Result<Self> {
        Ok(Self {
            vault_id: VaultId(parse_id(&row.vault_id)?),
            source_id: parse_opt_id(row.source_id)?,
            library_sample: row.library_sample.as_deref().map(json).transpose()?,
            cursor_modified: parse_opt_ts(row.cursor_modified)?,
            last_poll_at: parse_opt_ts(row.last_poll_at)?,
            last_full_reconcile_at: parse_opt_ts(row.last_full_reconcile_at)?,
            once_done: row.once_done,
            retry_ids: json(&row.retry_ids)?,
            parked_ids: json(&row.parked_ids)?,
            updated_at: parse_ts(&row.updated_at)?,
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
    conn: &mut SqliteConnection,
    entry: &PaperlessLedgerEntry,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO paperless_documents (vault_id, paperless_id, document_id, paperless_modified, checksum, fingerprint, needs_reapply, trashed_in_paperless, folio_state, last_synced_at, last_run_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT (vault_id, paperless_id) DO UPDATE SET document_id = excluded.document_id, paperless_modified = excluded.paperless_modified, checksum = excluded.checksum, fingerprint = excluded.fingerprint, needs_reapply = excluded.needs_reapply, trashed_in_paperless = excluded.trashed_in_paperless, folio_state = excluded.folio_state, last_synced_at = excluded.last_synced_at, last_run_id = excluded.last_run_id",
    )
    .bind(entry.vault_id.to_string())
    .bind(entry.paperless_id)
    .bind(entry.document_id.to_string())
    .bind(ts(entry.paperless_modified))
    .bind(&entry.checksum)
    .bind(&entry.fingerprint)
    .bind(entry.needs_reapply)
    .bind(entry.trashed_in_paperless)
    .bind(entry.folio_state.as_str())
    .bind(ts(entry.last_synced_at))
    .bind(entry.last_run_id.map(|id| id.to_string()))
    .execute(conn)
    .await
    .map_err(storage_error)?;
    Ok(())
}

/// Write the ledger row and finish the claimed item inside the caller's
/// transaction; a lost claim is a `Conflict`, so the caller's `?` rolls the
/// document change back
pub async fn record_import_applied(
    conn: &mut SqliteConnection,
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
    conn: &mut SqliteConnection,
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
        "UPDATE import_items SET status = ?1, reason = ?2, warnings = ?3, updated_at = ?4 \
         WHERE run_id = ?5 AND paperless_id = ?6 AND claim = ?7 AND status = 'in_progress'",
    )
    .bind(outcome.status.as_str())
    .bind(&outcome.reason)
    .bind(to_json(&outcome.warnings)?)
    .bind(ts(now))
    .bind(run.to_string())
    .bind(paperless_id)
    .bind(claim.to_string())
    .execute(conn)
    .await
    .map_err(storage_error)?;
    Ok(result.rows_affected() == 1)
}

/// Move a run between statuses inside the caller's transaction, so the
/// driver can copy the cursor into the sync state in the same transaction;
/// see [`ImportRunRepository::transition`]
pub async fn transition_import_run(
    conn: &mut SqliteConnection,
    id: Id,
    from: &[ImportRunStatus],
    to: ImportRunStatus,
    error: Option<String>,
    now: DateTime<Utc>,
) -> Result<RunTransition> {
    // A savepoint keeps the caller's transaction usable after the one-active
    // index rejects the update.
    let mut savepoint = conn.begin().await.map_err(storage_error)?;
    let updated = sqlx::query(
        "UPDATE import_runs SET status = ?1, \
             started_at = CASE WHEN ?1 = 'running' THEN ?2 ELSE started_at END, \
             finished_at = ?3, error = ?4 \
         WHERE id = ?5 AND status IN (SELECT value FROM json_each(?6))",
    )
    .bind(to.as_str())
    .bind(ts(now))
    .bind((!to.is_active()).then(|| ts(now)))
    .bind(error)
    .bind(id.to_string())
    .bind(to_json(
        &from
            .iter()
            .map(|status| status.as_str())
            .collect::<Vec<_>>(),
    )?)
    .execute(&mut *savepoint)
    .await;
    match updated {
        Ok(result) => {
            savepoint.commit().await.map_err(storage_error)?;
            Ok(if result.rows_affected() == 1 {
                RunTransition::Applied
            } else {
                RunTransition::Skipped
            })
        }
        Err(error)
            if error
                .as_database_error()
                .is_some_and(|e| e.is_unique_violation()) =>
        {
            savepoint.rollback().await.map_err(storage_error)?;
            let active: Option<String> = sqlx::query_scalar(
                "SELECT other.id FROM import_runs run \
                 JOIN import_runs other ON other.vault_id = run.vault_id AND other.id <> run.id \
                 WHERE run.id = ?1 AND other.status IN ('queued', 'running')",
            )
            .bind(id.to_string())
            .fetch_optional(&mut *conn)
            .await
            .map_err(storage_error)?;
            match active {
                Some(active_run) => Ok(RunTransition::Conflict {
                    active_run: parse_id(&active_run)?,
                }),
                None => Err(Error::Conflict(
                    "the vault's active import run changed meanwhile; try again".into(),
                )),
            }
        }
        Err(error) => Err(storage_error(error)),
    }
}

/// Write the vault's sync state inside the caller's transaction; see
/// [`PaperlessLedgerRepository::put_sync_state`]
pub async fn put_paperless_sync_state(
    conn: &mut SqliteConnection,
    state: &PaperlessSyncState,
    read_at: Option<DateTime<Utc>>,
) -> Result<()> {
    let sql = if read_at.is_some() {
        "UPDATE paperless_sync_state SET source_id = ?2, library_sample = ?3, cursor_modified = ?4, last_poll_at = ?5, last_full_reconcile_at = ?6, once_done = ?7, retry_ids = ?8, parked_ids = ?9, updated_at = ?10 \
         WHERE vault_id = ?1 AND updated_at = ?11"
    } else {
        "INSERT INTO paperless_sync_state (vault_id, source_id, library_sample, cursor_modified, last_poll_at, last_full_reconcile_at, once_done, retry_ids, parked_ids, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) ON CONFLICT (vault_id) DO NOTHING"
    };
    let mut query = sqlx::query(sql)
        .bind(state.vault_id.to_string())
        .bind(state.source_id.map(|id| id.to_string()))
        .bind(state.library_sample.as_ref().map(to_json).transpose()?)
        .bind(state.cursor_modified.map(ts))
        .bind(state.last_poll_at.map(ts))
        .bind(state.last_full_reconcile_at.map(ts))
        .bind(state.once_done)
        .bind(to_json(&state.retry_ids)?)
        .bind(to_json(&state.parked_ids)?)
        .bind(ts(state.updated_at));
    if let Some(read_at) = read_at {
        query = query.bind(ts(read_at));
    }
    let result = query.execute(conn).await.map_err(storage_error)?;
    if result.rows_affected() == 0 {
        return Err(Error::Conflict(
            "the sync state changed since it was read".into(),
        ));
    }
    Ok(())
}

#[async_trait]
impl ImportRunRepository for SqliteImportRepository {
    async fn create(&self, run: NewImportRun, now: DateTime<Utc>) -> Result<CreateImportRun> {
        // The active run can finish between a rejected insert and the lookup;
        // one more insert covers that.
        for _ in 0..2 {
            let inserted = sqlx::query_as::<_, RunRow>(sqlx::AssertSqlSafe(format!(
                r#"INSERT INTO import_runs (id, vault_id, kind, "trigger", source_id, source_path, dry_run, status, params, created_by, created_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued', ?8, ?9, ?10) RETURNING {RUN_COLUMNS}"#
            )))
            .bind(Id::new_v4().to_string())
            .bind(run.vault_id.to_string())
            .bind(run.kind.as_str())
            .bind(run.trigger.as_str())
            .bind(run.source_id.map(|id| id.to_string()))
            .bind(&run.source_path)
            .bind(run.dry_run)
            .bind(to_json(&run.params)?)
            .bind(run.created_by.as_ref().map(UserId::as_str))
            .bind(ts(now))
            .fetch_one(&self.pool)
            .await;
            match inserted {
                Ok(row) => return Ok(CreateImportRun::Created(Box::new(row.try_into()?))),
                Err(error)
                    if error
                        .as_database_error()
                        .is_some_and(|e| e.is_unique_violation()) =>
                {
                    let active: Option<String> = sqlx::query_scalar(
                        "SELECT id FROM import_runs WHERE vault_id = ?1 AND status IN ('queued', 'running')",
                    )
                    .bind(run.vault_id.to_string())
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(storage_error)?;
                    if let Some(active) = active {
                        return Ok(CreateImportRun::Conflict {
                            active_run: parse_id(&active)?,
                        });
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
            "SELECT {RUN_COLUMNS} FROM import_runs WHERE vault_id = ?1 AND id = ?2"
        )))
        .bind(vault.to_string())
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn list(&self, vault: VaultId, limit: u32) -> Result<Vec<ImportRun>> {
        sqlx::query_as::<_, RunRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {RUN_COLUMNS} FROM import_runs WHERE vault_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2"
        )))
        .bind(vault.to_string())
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
    ) -> Result<RunTransition> {
        let mut conn = self.pool.acquire().await.map_err(storage_error)?;
        transition_import_run(&mut conn, id, from, to, error, now).await
    }

    async fn set_plan_complete(&self, id: Id, complete: bool) -> Result<()> {
        self.update_run(
            sqlx::query("UPDATE import_runs SET plan_complete = ?1 WHERE id = ?2")
                .bind(complete)
                .bind(id.to_string()),
            id,
        )
        .await
    }

    async fn request_cancel(&self, id: Id) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE import_runs SET cancel_requested = 1 WHERE id = ?1 AND status IN ('queued', 'running')",
        )
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn increment_driver_restarts(&self, id: Id) -> Result<Option<u32>> {
        let restarts: Option<i64> = sqlx::query_scalar(
            "UPDATE import_runs SET driver_restarts = driver_restarts + 1 \
             WHERE id = ?1 AND status IN ('queued', 'running') RETURNING driver_restarts",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;
        restarts
            .map(|restarts| u32::try_from(restarts).map_err(storage_error))
            .transpose()
    }

    async fn set_cursor_after(&self, id: Id, cursor: Option<DateTime<Utc>>) -> Result<()> {
        self.update_run(
            sqlx::query("UPDATE import_runs SET cursor_after = ?1 WHERE id = ?2")
                .bind(cursor.map(ts))
                .bind(id.to_string()),
            id,
        )
        .await
    }

    async fn set_confirm_trash(&self, id: Id, confirm: bool) -> Result<()> {
        self.update_run(
            sqlx::query("UPDATE import_runs SET confirm_trash = ?1 WHERE id = ?2")
                .bind(confirm)
                .bind(id.to_string()),
            id,
        )
        .await
    }

    async fn set_summary(&self, id: Id, summary: serde_json::Value) -> Result<()> {
        self.update_run(
            sqlx::query("UPDATE import_runs SET summary = ?1 WHERE id = ?2")
                .bind(to_json(&summary)?)
                .bind(id.to_string()),
            id,
        )
        .await
    }
}

impl SqliteImportRepository {
    async fn update_run(
        &self,
        query: sqlx::query::Query<'_, sqlx::Sqlite, sqlx::sqlite::SqliteArguments>,
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
impl ImportItemRepository for SqliteImportRepository {
    async fn insert_items_ignore_existing(
        &self,
        run: Id,
        items: &[NewImportItem],
        now: DateTime<Utc>,
    ) -> Result<u64> {
        let (run, now) = (run.to_string(), ts(now));
        let mut tx = self.pool.begin().await.map_err(storage_error)?;
        let mut inserted = 0;
        for item in items {
            inserted += sqlx::query(
                "INSERT INTO import_items (run_id, paperless_id, action, document_id, title, status, reason, payload, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT (run_id, paperless_id) DO NOTHING",
            )
            .bind(&run)
            .bind(item.paperless_id)
            .bind(item.action.as_str())
            .bind(item.document_id.map(|id| id.to_string()))
            .bind(&item.title)
            .bind(item.status.as_str())
            .bind(&item.reason)
            .bind(item.payload.as_ref().map(to_json).transpose()?)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(storage_error)?
            .rows_affected();
        }
        tx.commit().await.map_err(storage_error)?;
        Ok(inserted)
    }

    async fn claim_next(
        &self,
        run: Id,
        claim: Id,
        now: DateTime<Utc>,
    ) -> Result<Option<ImportItem>> {
        sqlx::query_as::<_, ItemRow>(sqlx::AssertSqlSafe(format!(
            "UPDATE import_items SET status = 'in_progress', claim = ?1, claimed_at = ?2, attempts = attempts + 1, updated_at = ?2 \
             WHERE run_id = ?3 AND paperless_id = ( \
                 SELECT paperless_id FROM import_items \
                  WHERE run_id = ?3 AND status IN ('pending', 'in_progress') \
                    AND (status = 'pending' OR claimed_at < ?4) \
                  ORDER BY paperless_id LIMIT 1) \
             RETURNING {ITEM_COLUMNS}"
        )))
        .bind(claim.to_string())
        .bind(ts(now))
        .bind(run.to_string())
        .bind(ts(now - IMPORT_CLAIM_TIMEOUT))
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
            "UPDATE import_items SET status = 'pending', attempts = 0, claim = NULL, claimed_at = NULL, reason = NULL, warnings = '[]', updated_at = ?1 \
             WHERE run_id = ?2 AND status = 'failed'",
        )
        .bind(ts(now))
        .bind(run.to_string())
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(result.rows_affected())
    }

    async fn counts(&self, run: Id) -> Result<Vec<ImportItemCount>> {
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT action, status, COUNT(*) FROM import_items WHERE run_id = ?1 GROUP BY action, status ORDER BY action, status",
        )
        .bind(run.to_string())
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
            "AND status = ?4"
        } else {
            ""
        };
        let mut query = sqlx::query_as::<_, ItemRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {ITEM_COLUMNS} FROM import_items \
             WHERE run_id = ?1 AND paperless_id > ?2 {filter} \
             ORDER BY paperless_id LIMIT ?3"
        )))
        .bind(run.to_string())
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
impl PaperlessLedgerRepository for SqliteImportRepository {
    async fn get_document(
        &self,
        vault: VaultId,
        paperless_id: i64,
    ) -> Result<Option<PaperlessLedgerEntry>> {
        sqlx::query_as::<_, LedgerRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {LEDGER_COLUMNS} FROM paperless_documents WHERE vault_id = ?1 AND paperless_id = ?2"
        )))
        .bind(vault.to_string())
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
            "SELECT {LEDGER_COLUMNS} FROM paperless_documents WHERE vault_id = ?1 AND paperless_id > ?2 ORDER BY paperless_id LIMIT ?3"
        )))
        .bind(vault.to_string())
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
            sqlx::AssertSqlSafe(format!("SELECT {TAXONOMY_COLUMNS} FROM paperless_taxonomy WHERE vault_id = ?1 AND kind = ?2 AND paperless_id = ?3")),
        )
        .bind(vault.to_string())
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
            sqlx::AssertSqlSafe(format!("SELECT {TAXONOMY_COLUMNS} FROM paperless_taxonomy WHERE vault_id = ?1 AND kind = ?2 ORDER BY paperless_id")),
        )
        .bind(vault.to_string())
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
            "INSERT INTO paperless_taxonomy (vault_id, kind, paperless_id, entity_id, name) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (vault_id, kind, paperless_id) DO UPDATE SET entity_id = excluded.entity_id, name = excluded.name",
        )
        .bind(mapping.vault_id.to_string())
        .bind(mapping.kind.as_str())
        .bind(mapping.paperless_id)
        .bind(mapping.entity_id.to_string())
        .bind(&mapping.name)
        .execute(&self.pool)
        .await
        .map_err(storage_error)?;
        Ok(())
    }

    async fn sync_state(&self, vault: VaultId) -> Result<Option<PaperlessSyncState>> {
        sqlx::query_as::<_, SyncStateRow>(
            "SELECT vault_id, source_id, library_sample, cursor_modified, last_poll_at, last_full_reconcile_at, once_done, retry_ids, parked_ids, updated_at \
             FROM paperless_sync_state WHERE vault_id = ?1",
        )
        .bind(vault.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn put_sync_state(
        &self,
        state: &PaperlessSyncState,
        read_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        let mut conn = self.pool.acquire().await.map_err(storage_error)?;
        put_paperless_sync_state(&mut conn, state, read_at).await
    }
}
