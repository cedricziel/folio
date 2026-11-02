//! Vault provisioning and default-vault resolution

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::Result;
use crate::models::{Principal, UserId, VaultId, VaultRole};
use crate::ports::{MembershipRepository, VaultRepository};

/// Name of the vault every user gets for their own papers
pub const PERSONAL_NAME: &str = "Personal";

/// Gives users their vaults and resolves which vault a request without an
/// explicit one lands in
pub struct VaultService {
    vaults: Arc<dyn VaultRepository>,
    members: Arc<dyn MembershipRepository>,
    /// Serializes provisioning so two first requests cannot create two `Personal` vaults
    provisioning: Mutex<()>,
}

impl VaultService {
    pub fn new(vaults: Arc<dyn VaultRepository>, members: Arc<dyn MembershipRepository>) -> Self {
        Self {
            vaults,
            members,
            provisioning: Mutex::new(()),
        }
    }

    /// Make sure the user owns a `Personal` vault and has a default vault
    ///
    /// Preserve an existing default. Safe to call on every request.
    pub async fn provision_user(&self, user: &UserId) -> Result<VaultId> {
        let _guard = self.provisioning.lock().await;
        if self.vaults.provisioned(user).await? {
            if let Some(vault) = self.vaults.default_for_user(user).await? {
                return Ok(vault);
            }
            return self
                .vaults
                .list_for_user(user)
                .await?
                .first()
                .map(|(vault, _)| vault.id)
                .ok_or_else(|| crate::error::Error::Repository("no default vault".into()));
        }
        let had_default = self.vaults.default_for_user(user).await?;

        let existing_personal = self
            .vaults
            .list_for_user(user)
            .await?
            .into_iter()
            .find(|(v, role)| {
                v.name == PERSONAL_NAME
                    && *role == VaultRole::Owner
                    && v.created_by.as_ref() == Some(user)
            })
            .map(|(v, _)| v.id);
        let personal = if let Some(existing) = existing_personal {
            existing
        } else {
            let vault = self.vaults.create(PERSONAL_NAME, Some(user)).await?;
            self.members
                .grant(
                    vault.id,
                    &Principal::User(user.clone()),
                    VaultRole::Owner,
                    Some(user),
                )
                .await?;
            vault.id
        };
        let wanted = had_default.unwrap_or(personal);
        self.vaults.set_default(user, wanted).await?;
        self.vaults.mark_provisioned(user).await?;
        Ok(wanted)
    }

    /// The vault a request lands in: the user's default (provisioned on first
    /// use), or the instance default when there is no user
    pub async fn default_vault(&self, user: Option<&UserId>) -> Result<VaultId> {
        match user {
            Some(user) => self.provision_user(user).await,
            None => self.vaults.instance_default().await,
        }
    }
}
