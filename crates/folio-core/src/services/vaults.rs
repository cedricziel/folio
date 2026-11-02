//! Vault provisioning and default-vault resolution

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::Result;
use crate::models::{Principal, UserId, VaultId, VaultRole};
use crate::ports::{MembershipRepository, VaultRepository};

/// Name of the vault that receives pre-existing data on upgrade
pub const ARCHIVE_NAME: &str = "Archive";
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
    /// The instance owner also claims every `Archive` that has no direct owner
    /// (the upgrade creates it before any user exists), and that becomes their
    /// default. Safe to call on every request: it does nothing once done.
    pub async fn provision_user(&self, user: &UserId, is_owner: bool) -> Result<VaultId> {
        let _guard = self.provisioning.lock().await;
        if self.vaults.provisioned(user).await? {
            return self
                .vaults
                .default_for_user(user)
                .await?
                .ok_or_else(|| crate::error::Error::Repository("no default vault".into()));
        }
        let had_default = self.vaults.default_for_user(user).await?;

        let mut claimed = None;
        if is_owner {
            for archive in self.vaults.find_ownerless(ARCHIVE_NAME).await? {
                self.members
                    .grant(
                        archive,
                        &Principal::User(user.clone()),
                        VaultRole::Owner,
                        None,
                    )
                    .await?;
                claimed.get_or_insert(archive);
            }
        }

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
        let migrated_archive = if is_owner {
            match had_default {
                Some(id) => self
                    .vaults
                    .find(id)
                    .await?
                    .filter(|v| v.name == ARCHIVE_NAME)
                    .map(|v| v.id),
                None => None,
            }
        } else {
            None
        };
        let wanted = claimed.or(migrated_archive).unwrap_or(personal);
        self.vaults.set_default(user, wanted).await?;
        self.vaults.mark_provisioned(user).await?;
        Ok(wanted)
    }

    /// The vault a request lands in: the user's default (provisioned on first
    /// use), or the instance default when there is no user
    pub async fn default_vault(&self, user: Option<(&UserId, bool)>) -> Result<VaultId> {
        match user {
            Some((user, is_owner)) => self.provision_user(user, is_owner).await,
            None => self.vaults.instance_default().await,
        }
    }
}
