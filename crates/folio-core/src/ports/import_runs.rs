//! Storage for Paperless import runs, their items and the sync ledger

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::models::{
    CreateImportRun, Id, ImportItem, ImportItemCount, ImportItemOutcome, ImportItemStatus,
    ImportRun, ImportRunStatus, NewImportItem, NewImportRun, PaperlessLedgerEntry,
    PaperlessSyncState, RunTransition, TaxonomyKind, TaxonomyMapping, VaultId,
};

#[async_trait]
pub trait ImportRunRepository: Send + Sync {
    /// Create a queued run, unless the vault already has a queued or running one
    async fn create(&self, run: NewImportRun, now: DateTime<Utc>) -> Result<CreateImportRun>;

    async fn get(&self, vault: VaultId, id: Id) -> Result<Option<ImportRun>>;

    /// Runs of a vault, newest first
    async fn list(&self, vault: VaultId, limit: u32) -> Result<Vec<ImportRun>>;

    /// Move the run to `to` only when its status is one of `from`. Entering
    /// `running` sets `started_at`, entering a final status sets
    /// `finished_at`, and `error` replaces the stored error. Re-entering an
    /// active status while another run in the vault is active is a
    /// [`RunTransition::Conflict`]. To change the sync state in the same
    /// transaction, use the backend's `transition_import_run`.
    async fn transition(
        &self,
        id: Id,
        from: &[ImportRunStatus],
        to: ImportRunStatus,
        error: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<RunTransition>;

    async fn set_plan_complete(&self, id: Id, complete: bool) -> Result<()>;

    /// Flag a queued or running run for cancellation; returns whether it was
    /// flagged
    async fn request_cancel(&self, id: Id) -> Result<bool>;

    /// Returns the new restart count, or `None` when the run isn't queued or
    /// running
    async fn increment_driver_restarts(&self, id: Id) -> Result<Option<u32>>;

    async fn set_cursor_after(&self, id: Id, cursor: Option<DateTime<Utc>>) -> Result<()>;

    async fn set_confirm_trash(&self, id: Id, confirm: bool) -> Result<()>;

    async fn set_summary(&self, id: Id, summary: serde_json::Value) -> Result<()>;
}

#[async_trait]
pub trait ImportItemRepository: Send + Sync {
    /// Insert items that the run doesn't have yet; returns how many were new
    async fn insert_items_ignore_existing(
        &self,
        run: Id,
        items: &[NewImportItem],
        now: DateTime<Utc>,
    ) -> Result<u64>;

    /// Claim the lowest pending item, or one whose claim is older than
    /// [`crate::models::IMPORT_CLAIM_TIMEOUT`], counting the attempt; `None`
    /// when nothing is claimable
    async fn claim_next(
        &self,
        run: Id,
        claim: Id,
        now: DateTime<Utc>,
    ) -> Result<Option<ImportItem>>;

    /// Record the item's outcome while `claim` still holds it; returns
    /// whether it did
    async fn finish_item(
        &self,
        run: Id,
        paperless_id: i64,
        claim: Id,
        outcome: ImportItemOutcome,
        now: DateTime<Utc>,
    ) -> Result<bool>;

    /// Return failed items to `pending` with no attempts and no claim;
    /// returns how many were reset. Callers must first make sure the run
    /// isn't queued or running, or a driver may skip the reset items.
    async fn reset_failed_items(&self, run: Id, now: DateTime<Utc>) -> Result<u64>;

    async fn counts(&self, run: Id) -> Result<Vec<ImportItemCount>>;

    /// Items in Paperless ID order after `after`
    async fn list_items(
        &self,
        run: Id,
        status: Option<ImportItemStatus>,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<ImportItem>>;
}

/// The sync ledger, the taxonomy map and the per-vault sync state. Document
/// changes write their ledger row in their own transaction through the
/// backend's `upsert_paperless_document` function instead.
#[async_trait]
pub trait PaperlessLedgerRepository: Send + Sync {
    async fn get_document(
        &self,
        vault: VaultId,
        paperless_id: i64,
    ) -> Result<Option<PaperlessLedgerEntry>>;

    /// Ledger rows in Paperless ID order after `after`
    async fn list_documents(
        &self,
        vault: VaultId,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<PaperlessLedgerEntry>>;

    async fn upsert_document(&self, entry: &PaperlessLedgerEntry) -> Result<()>;

    async fn get_taxonomy(
        &self,
        vault: VaultId,
        kind: TaxonomyKind,
        paperless_id: i64,
    ) -> Result<Option<TaxonomyMapping>>;

    /// Every mapping of one kind, in Paperless ID order
    async fn taxonomy_map(
        &self,
        vault: VaultId,
        kind: TaxonomyKind,
    ) -> Result<Vec<TaxonomyMapping>>;

    async fn upsert_taxonomy(&self, mapping: &TaxonomyMapping) -> Result<()>;

    async fn sync_state(&self, vault: VaultId) -> Result<Option<PaperlessSyncState>>;

    /// Replace the vault's sync state, but only if it is unchanged since it
    /// was read: `read_at` is the `updated_at` the caller read, or `None` when
    /// there was no state yet. Anything else is a `Conflict`; read again and
    /// retry. To write it in the caller's transaction, use the backend's
    /// `put_paperless_sync_state`.
    async fn put_sync_state(
        &self,
        state: &PaperlessSyncState,
        read_at: Option<DateTime<Utc>>,
    ) -> Result<()>;
}
