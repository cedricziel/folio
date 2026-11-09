//! Vault provisioning and default-vault resolution

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::Result;
use crate::models::{AuditEvent, Principal, UserId, Vault, VaultId, VaultMember, VaultRole};
use crate::ports::{AuditLog, MembershipRepository, VaultRepository};

/// Name of the vault every user gets for their own papers
pub const PERSONAL_NAME: &str = "Personal";

/// Gives users their vaults and resolves which vault a request without an
/// explicit one lands in
pub struct VaultService {
    vaults: Arc<dyn VaultRepository>,
    members: Arc<dyn MembershipRepository>,
    audit: Option<Arc<dyn AuditLog>>,
    /// Serializes provisioning so two first requests cannot create two `Personal` vaults
    provisioning: Mutex<()>,
}

impl VaultService {
    pub fn new(vaults: Arc<dyn VaultRepository>, members: Arc<dyn MembershipRepository>) -> Self {
        Self {
            vaults,
            members,
            audit: None,
            provisioning: Mutex::new(()),
        }
    }

    /// Configure permanent owner-visible lifecycle history.
    pub fn with_audit(mut self, audit: Arc<dyn AuditLog>) -> Self {
        self.audit = Some(audit);
        self
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
            let fallback = self
                .vaults
                .list_for_user(user)
                .await?
                .first()
                .map(|(vault, _)| vault.id)
                .ok_or_else(|| crate::error::Error::Repository("no default vault".into()))?;
            self.vaults.set_default(user, fallback).await?;
            return Ok(fallback);
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

    /// Lists only vaults visible to the user; unauthenticated mode exposes the instance vault.
    pub async fn list(&self, user: Option<&UserId>) -> Result<Vec<(Vault, VaultRole)>> {
        match user {
            Some(user) => self.vaults.list_for_user(user).await,
            None => {
                let id = self.vaults.instance_default().await?;
                let vault = self.vaults.find(id).await?.ok_or_else(|| {
                    crate::error::Error::Repository("instance vault is missing".into())
                })?;
                Ok(vec![(vault, VaultRole::Owner)])
            }
        }
    }

    /// Those of `vaults` with a grant to anyone besides `user`, in one query. A group grant counts
    /// as shared, even when `user` is its only member. Call it only for vaults [`Self::list`]
    /// returned for `user`.
    pub async fn shared_beyond(
        &self,
        user: &UserId,
        vaults: &[VaultId],
    ) -> Result<std::collections::HashSet<VaultId>> {
        self.members.shared_beyond(user, vaults).await
    }

    /// The vault a request lands in: the user's default (provisioned on first
    /// use), or the instance default when there is no user
    pub async fn default_vault(&self, user: Option<&UserId>) -> Result<VaultId> {
        match user {
            Some(user) => self.provision_user(user).await,
            None => self.vaults.instance_default().await,
        }
    }

    /// Return retained vault metadata only to a current owner.
    pub async fn deleted(&self, user: &UserId) -> Result<Vec<Vault>> {
        self.vaults.list_deleted_for_user(user).await
    }

    /// Return a deleted vault only when the user still owns it.
    pub async fn retained_for_owner(&self, user: &UserId, id: VaultId) -> Result<Option<Vault>> {
        Ok(self
            .deleted(user)
            .await?
            .into_iter()
            .find(|vault| vault.id == id))
    }

    /// Create a vault and an initial owner using server mutation attribution.
    pub async fn create(&self, user: &UserId, name: &str) -> Result<Vault> {
        self.vaults.create(name, Some(user)).await
    }

    /// Return active vault metadata after the request's access check.
    pub async fn find(&self, id: VaultId) -> Result<Option<Vault>> {
        self.vaults.find(id).await
    }

    async fn require_owner(&self, user: &UserId, id: VaultId, deleted: bool) -> Result<()> {
        if self
            .vaults
            .list_for_user(user)
            .await?
            .iter()
            .any(|(vault, role)| vault.id == id && *role == VaultRole::Owner)
            || (deleted && self.retained_for_owner(user, id).await?.is_some())
        {
            return Ok(());
        }
        Err(crate::Error::NotFound {
            entity_type: "Vault",
            id: id.to_string(),
        })
    }

    /// Rename an active vault after rechecking current ownership.
    pub async fn rename(&self, user: &UserId, id: VaultId, name: &str) -> Result<Vault> {
        self.require_owner(user, id, false).await?;
        self.vaults.rename(id, name).await
    }

    /// Retain a vault after rechecking current ownership.
    pub async fn delete(&self, user: &UserId, id: VaultId) -> Result<()> {
        self.require_owner(user, id, true).await?;
        self.vaults.delete(id).await
    }

    /// Restore a vault only while its retained grants still make the user an owner.
    pub async fn restore(&self, user: &UserId, id: VaultId) -> Result<Vault> {
        self.require_owner(user, id, true).await?;
        self.vaults.restore(id).await
    }

    /// Select an active vault to which the user currently has access.
    pub async fn set_default(&self, user: &UserId, id: VaultId) -> Result<()> {
        self.vaults.set_default(user, id).await
    }

    /// List grants only while the user currently owns the active vault.
    pub async fn members(&self, user: &UserId, id: VaultId) -> Result<Vec<VaultMember>> {
        self.require_owner(user, id, false).await?;
        self.members.list(id).await
    }

    /// Change a current grant after rechecking ownership.
    pub async fn grant(
        &self,
        user: &UserId,
        id: VaultId,
        principal: &Principal,
        role: VaultRole,
    ) -> Result<()> {
        self.require_owner(user, id, false).await?;
        self.members.grant(id, principal, role, Some(user)).await
    }

    /// Revoke a grant while retaining the last direct-owner protection.
    pub async fn revoke(&self, user: &UserId, id: VaultId, principal: &Principal) -> Result<()> {
        self.require_owner(user, id, false).await?;
        self.members.revoke(id, principal).await
    }

    /// Read retained lifecycle facts only while the user is a current owner.
    pub async fn history(&self, user: &UserId, id: VaultId) -> Result<Vec<AuditEvent>> {
        self.require_owner(user, id, true).await?;
        let audit = self
            .audit
            .as_ref()
            .ok_or_else(|| crate::Error::Repository("Vault history is not configured".into()))?;
        audit.list(id, 100).await
    }
}
