//! Storage for Paperless import runs, their items and the sync ledger

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::models::{
    CreateImportRun, Id, ImportItem, ImportItemCount, ImportItemOutcome, ImportItemStatus,
    ImportRun, ImportRunStatus, NewImportItem, NewImportRun, PaperlessLedgerEntry,
    PaperlessSyncState, TaxonomyKind, TaxonomyMapping, VaultId,
};

#[async_trait]
pub trait ImportRunRepository: Send + Sync {
    /// Create a queued run, unless the vault already has a queued or running one
    async fn create(&self, run: NewImportRun) -> Result<CreateImportRun>;

    async fn get(&self, vault: VaultId, id: Id) -> Result<Option<ImportRun>>;

    /// Runs of a vault, newest first
    async fn list(&self, vault: VaultId, limit: u32) -> Result<Vec<ImportRun>>;

    /// Move the run to `to` only when its status is one of `from`; returns
    /// whether it moved. Entering `running` sets `started_at`, entering a
    /// final status sets `finished_at`, and `error` replaces the stored error.
    async fn transition(
        &self,
        id: Id,
        from: &[ImportRunStatus],
        to: ImportRunStatus,
        error: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<bool>;

    async fn set_plan_complete(&self, id: Id, complete: bool) -> Result<()>;

    /// Flag a queued or running run for cancellation; returns whether it was
    /// flagged
    async fn request_cancel(&self, id: Id) -> Result<bool>;

    /// Returns the new restart count
    async fn increment_driver_restarts(&self, id: Id) -> Result<u32>;

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
    /// returns how many were reset
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

    /// Replace the vault's sync state
    async fn put_sync_state(&self, state: &PaperlessSyncState) -> Result<()>;
}
