use async_trait::async_trait;

use crate::error::Result;
use crate::models::{Id, SavedView, SavedViewInput, UserId, VaultId, VaultRole};

/// Authenticated identity and freshly resolved vault role for a saved-view operation.
pub struct SavedViewAccess {
    pub user: UserId,
    pub role: VaultRole,
}

#[async_trait]
pub trait SavedViewRepository: Send + Sync {
    /// List visible live views after an optional ID, with a limit of 100 and an optional query version ceiling.
    async fn list(
        &self,
        vault: VaultId,
        access: &SavedViewAccess,
        after: Option<Id>,
        limit: u32,
        max_query_version: Option<u32>,
    ) -> Result<Vec<SavedView>>;
    /// Return a live view only when it is visible to the current vault member.
    async fn find(
        &self,
        vault: VaultId,
        access: &SavedViewAccess,
        id: Id,
    ) -> Result<Option<SavedView>>;
    /// Check the current visibility of a scoped live view or retained deletion for workflow delivery.
    async fn is_shared_for_workflow(&self, vault: VaultId, id: Id, deleted: bool) -> Result<bool>;
    /// Create a view owned by the authenticated member, deriving lifecycle metadata.
    async fn create(
        &self,
        vault: VaultId,
        access: &SavedViewAccess,
        input: SavedViewInput,
    ) -> Result<SavedView>;
    /// Replace editable properties while preserving ownership and creation metadata.
    async fn update(
        &self,
        vault: VaultId,
        access: &SavedViewAccess,
        id: Id,
        input: SavedViewInput,
    ) -> Result<SavedView>;
    /// Soft-delete an authorized view and record the authenticated actor.
    async fn delete(&self, vault: VaultId, access: &SavedViewAccess, id: Id) -> Result<()>;
}
