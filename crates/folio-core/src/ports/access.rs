//! Access decisions for vaults

use async_trait::async_trait;

use crate::error::Result;
use crate::models::{Access, UserId, VaultAction, VaultId, VaultRole};

/// Decides what a user may do in a vault
///
/// Instance admins have no implicit role: implementations must not consult an
/// admin flag.
#[async_trait]
pub trait AccessPolicy: Send + Sync {
    /// The highest role among the user's direct grant and the grants of every
    /// group they belong to right now; `None` when they have no role at all.
    async fn effective_role(&self, user: &UserId, vault: VaultId) -> Result<Option<VaultRole>>;

    async fn can(&self, user: &UserId, vault: VaultId, action: VaultAction) -> Result<Access> {
        Ok(match self.effective_role(user, vault).await? {
            None => Access::NotVisible,
            Some(role) if role.permits(&action, user) => Access::Allowed(role),
            Some(role) => Access::Forbidden(role),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub(Option<VaultRole>);

    #[async_trait]
    impl AccessPolicy for Stub {
        async fn effective_role(&self, _: &UserId, _: VaultId) -> Result<Option<VaultRole>> {
            Ok(self.0)
        }
    }

    #[tokio::test]
    async fn can_maps_roles_to_access() {
        let user = UserId::from("alice");
        let vault = VaultId::new();
        let check = |role, action| {
            let user = user.clone();
            async move { Stub(role).can(&user, vault, action).await.unwrap() }
        };

        assert_eq!(check(None, VaultAction::Read).await, Access::NotVisible);
        assert_eq!(
            check(Some(VaultRole::Viewer), VaultAction::Read).await,
            Access::Allowed(VaultRole::Viewer)
        );
        assert_eq!(
            check(Some(VaultRole::Viewer), VaultAction::ManageVocabulary).await,
            Access::Forbidden(VaultRole::Viewer)
        );
        assert_eq!(
            check(Some(VaultRole::Owner), VaultAction::DeleteVault).await,
            Access::Allowed(VaultRole::Owner)
        );
    }
}
