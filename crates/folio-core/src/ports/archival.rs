use async_trait::async_trait;

use crate::models::{ArchiveAttempt, ArchiveCompletion, ArchiveSource, Id, VaultId};
use crate::Result;

/// Revision-bound archival state. Callers enforce user access before reading this port.
#[async_trait]
pub trait ArchivalRepository: Send + Sync {
    /// The live verified original. Unverified uploads, Trash and purging documents are invisible.
    async fn current_source(&self, vault: VaultId, document: Id) -> Result<Option<ArchiveSource>>;

    /// The current attempt for the live verified original, including its state.
    /// Resolve this per download request because revision and attempt changes hide old results.
    async fn current_attempt(&self, vault: VaultId, document: Id)
        -> Result<Option<ArchiveAttempt>>;

    /// Compare-and-swap queueing. Use a fresh ID, and pass the observed current attempt ID
    /// on retry (or None for the initial attempt). Replaying the current ID succeeds.
    /// The server adapter commits the durable delivery alongside the new attempt.
    /// False means the source or expected attempt changed.
    async fn enqueue_archive(
        &self,
        source: &ArchiveSource,
        attempt: Id,
        expected_attempt: Option<Id>,
    ) -> Result<bool>;

    /// Complete a queued attempt for the matching live source. An identical replay of
    /// the current terminal result succeeds. False means the attempt cannot publish.
    ///
    /// Verify the original bytes against `source.original` before conversion.
    /// For Ready, validate the exact output bytes and store them create-only at
    /// `keys::revision_artifact(document, revision, attempt)` before calling.
    /// Only the worker owning the staged write may consider cleanup. After ANY
    /// rejected or ambiguous publication, call `retains_archive` before deleting:
    /// a rejected replay can follow a successful commit now hidden by a retry or
    /// lifecycle change. Retain the bytes if that check succeeds or fails to resolve.
    /// Workers must also fence off any other publisher using that attempt ID
    /// before checking retention and cleaning up. Committed objects are retained for
    /// document-prefix purge, including if a newer attempt replaces them.
    async fn complete_archive(
        &self,
        source: &ArchiveSource,
        attempt: Id,
        completion: &ArchiveCompletion,
    ) -> Result<bool>;

    /// Check for a committed object after any rejected or ambiguous publication, even if a retry,
    /// replacement, move or trash operation has since hidden the result.
    async fn retains_archive(&self, document: Id, revision: Id, attempt: Id) -> Result<bool>;
}
