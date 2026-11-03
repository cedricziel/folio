use async_trait::async_trait;

use crate::models::{Id, MetadataField, MetadataFieldInput, VaultId};
use crate::Result;

/// Callers authorize vault access; every repository operation retains that vault scope.
#[async_trait]
pub trait MetadataFieldRepository: Send + Sync {
    /// Read at most 100 vault-owned definitions, ordered by stable ID.
    async fn list(
        &self,
        vault: VaultId,
        include_deleted: bool,
        after: Option<Id>,
        limit: u32,
    ) -> Result<Vec<MetadataField>>;
    /// Read a retained definition without resolving IDs outside the supplied vault.
    async fn find(&self, vault: VaultId, id: Id) -> Result<Option<MetadataField>>;
    /// Derive identity and attribution on the server and publish creation atomically.
    async fn create(&self, vault: VaultId, input: MetadataFieldInput) -> Result<MetadataField>;
    /// Replace editable properties, preserving creation metadata and suppressing no-op events.
    async fn update(
        &self,
        vault: VaultId,
        id: Id,
        input: MetadataFieldInput,
    ) -> Result<MetadataField>;
    /// Retire or restore idempotently; restoration conflicts when a live name was reused.
    async fn set_deleted(&self, vault: VaultId, id: Id, deleted: bool) -> Result<MetadataField>;
}
