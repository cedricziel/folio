use async_trait::async_trait;

use crate::error::Result;
use crate::models::{DocumentEntry, EntryBody, EntryKind, EntryPage, EntryPageQuery, Id, VaultId};

#[derive(Debug)]
pub enum EntryCreation {
    Created(DocumentEntry),
    Replayed(DocumentEntry),
    Deleted,
}

/// Scoped entry persistence; mutations derive attribution from the server mutation context.
/// Callers authorize current vault membership. Adapters enforce live parent scope and authorship,
/// and commit entries, creation receipts, and outbox transitions atomically.
#[async_trait]
pub trait DocumentEntryRepository: Send + Sync {
    async fn find(&self, vault: VaultId, document: Id, entry: Id) -> Result<Option<DocumentEntry>>;

    async fn page(&self, vault: VaultId, document: Id, query: EntryPageQuery) -> Result<EntryPage>;

    /// A matching creation key replays its receipt; a different payload conflicts.
    /// A receipt for a soft-deleted entry returns Deleted without exposing its retained body.
    async fn create(
        &self,
        vault: VaultId,
        document: Id,
        kind: EntryKind,
        body: EntryBody,
        idempotency_key: Id,
    ) -> Result<EntryCreation>;

    /// Update only the authenticated author's live entry; identical text emits no event.
    async fn update(
        &self,
        vault: VaultId,
        document: Id,
        entry: Id,
        body: EntryBody,
    ) -> Result<DocumentEntry>;

    /// Soft-delete an author's entry or a moderated entry; repeated deletion emits no event.
    /// allow_moderation must come from current server authorization, never a request field.
    async fn delete(
        &self,
        vault: VaultId,
        document: Id,
        entry: Id,
        allow_moderation: bool,
    ) -> Result<()>;
}
