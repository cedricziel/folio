//! Ports for automatic ingestion

use async_trait::async_trait;

use crate::error::Result;
use crate::models::{
    Document, Id, IngestItem, IngestOutcome, IngestionSourceDef, IngestionSourceInput, LedgerEntry,
    PollReport, SourceStatus, VaultId,
};
use crate::services::NewUpload;

/// Receives items found by a source
#[async_trait]
pub trait IngestSink: Send + Sync {
    async fn ingest(&self, source_id: Id, item: IngestItem) -> Result<IngestOutcome>;
}

/// Receives items for a known vault; the scheduler binds each source to its
/// vault and hands the result to the source as an [`IngestSink`]
#[async_trait]
pub trait VaultIngest: Send + Sync {
    async fn ingest(
        &self,
        vault: VaultId,
        source_id: Id,
        item: IngestItem,
    ) -> Result<IngestOutcome>;
}

/// A place documents arrive from, such as a folder or a mailbox
#[async_trait]
pub trait IngestionSource: Send + Sync {
    /// Look for new items once and hand each to `sink`
    async fn poll(&self, sink: &dyn IngestSink) -> Result<PollReport>;

    /// Resolves when the source has a reason to be polled before its interval ends
    async fn wake(&self) {
        std::future::pending::<()>().await
    }

    /// Check that the source is reachable and usable; returns a short summary
    async fn test(&self) -> Result<String>;

    /// Seconds between regular polls
    fn interval_secs(&self) -> u64;
}

/// Builds the adapter for a source definition
pub trait IngestionSourceFactory: Send + Sync {
    fn build(&self, def: &IngestionSourceDef) -> Result<Box<dyn IngestionSource>>;
}

/// The single path that turns bytes into a stored document
///
/// Creates the record (which announces `document.uploaded`), stores the file,
/// and schedules background processing.
#[async_trait]
pub trait DocumentImporter: Send + Sync {
    async fn import(&self, vault: VaultId, upload: NewUpload) -> Result<Document>;
    /// Resume an import using the document identity durably reserved by `claim`.
    async fn import_reserved(
        &self,
        vault: VaultId,
        claim: &LedgerEntry,
        upload: NewUpload,
    ) -> Result<Document>;
}

/// Persistence for managed sources and the status of every source
#[async_trait]
pub trait SourceRepository: Send + Sync {
    /// Active managed sources within the vault.
    async fn list(&self, vault: VaultId) -> Result<Vec<IngestionSourceDef>>;
    /// Every active managed source of every vault, for the scheduler
    async fn list_all(&self) -> Result<Vec<IngestionSourceDef>>;
    /// Retained managed sources excluded from normal reads and scheduling.
    async fn list_deleted(&self, vault: VaultId) -> Result<Vec<IngestionSourceDef>>;
    /// Look up a retained source only within its vault.
    async fn find_deleted(&self, vault: VaultId, id: Id) -> Result<Option<IngestionSourceDef>>;
    /// Restore the same identity; an active name collision returns a duplicate error.
    async fn restore(&self, vault: VaultId, id: Id) -> Result<IngestionSourceDef>;
    /// An active managed source; deleted sources are hidden.
    async fn find(&self, vault: VaultId, id: Id) -> Result<Option<IngestionSourceDef>>;
    async fn create(
        &self,
        vault: VaultId,
        input: IngestionSourceInput,
    ) -> Result<IngestionSourceDef>;
    async fn update(
        &self,
        vault: VaultId,
        id: Id,
        input: IngestionSourceInput,
    ) -> Result<IngestionSourceDef>;
    /// Move to trash without dropping status or ledger history; repeated deletion is a no-op.
    async fn delete(&self, vault: VaultId, id: Id) -> Result<()>;
    async fn save_status(&self, id: Id, status: &SourceStatus) -> Result<()>;
    async fn status(&self, id: Id) -> Result<Option<SourceStatus>>;
}

/// What the pipeline already handled, so nothing is imported twice
///
/// Keys are unique per vault, so the same file can be imported into two
/// vaults; `insert` must not overwrite an existing entry.
#[async_trait]
pub trait IngestLedger: Send + Sync {
    /// Returns whether the entry was new
    async fn insert(&self, vault: VaultId, entry: &LedgerEntry) -> Result<bool>;
    async fn find(&self, vault: VaultId, key: &str) -> Result<Option<LedgerEntry>>;
    async fn complete(
        &self,
        vault: VaultId,
        key: &str,
        outcome: &str,
        document_id: Option<Id>,
    ) -> Result<()>;
    /// Change only the unchanged reservation, fencing late import attempts.
    async fn transition_if_matches(
        &self,
        vault: VaultId,
        expected: &LedgerEntry,
        outcome: &str,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool>;
    /// Delete only the unchanged entry; returns false when it changed or disappeared.
    async fn delete_if_matches(&self, vault: VaultId, expected: &LedgerEntry) -> Result<bool>;
    async fn delete(&self, vault: VaultId, key: &str) -> Result<()>;
}
