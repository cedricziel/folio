use async_trait::async_trait;

use crate::models::{
    Id, MetadataField, MetadataFieldInput, MetadataOption, MetadataOptionInput, VaultId,
};
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

/// Callers authorize vault access; mutations require an active select field and publish atomically.
#[async_trait]
pub trait MetadataOptionRepository: Send + Sync {
    /// Read up to 100 retained options in stable ID order; position controls display order.
    async fn list(
        &self,
        vault: VaultId,
        field: Id,
        include_deleted: bool,
        after: Option<Id>,
        limit: u32,
    ) -> Result<Vec<MetadataOption>>;
    /// Read a retained option only within the supplied field and vault.
    async fn find(&self, vault: VaultId, field: Id, id: Id) -> Result<Option<MetadataOption>>;
    /// Derive identity and attribution on the server; live labels are unique within the field.
    async fn create(
        &self,
        vault: VaultId,
        field: Id,
        input: MetadataOptionInput,
    ) -> Result<MetadataOption>;
    /// Replace label and position without changing identity; unchanged input emits no event.
    async fn update(
        &self,
        vault: VaultId,
        field: Id,
        id: Id,
        input: MetadataOptionInput,
    ) -> Result<MetadataOption>;
    /// Retire or restore idempotently; restoration checks live label conflicts.
    async fn set_deleted(
        &self,
        vault: VaultId,
        field: Id,
        id: Id,
        deleted: bool,
    ) -> Result<MetadataOption>;
}
