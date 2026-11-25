use std::collections::HashMap;
use std::sync::Arc;

use crate::models::{Id, IngestionSourceDef, SourceKind, SourceOrigin, VaultId};
use crate::ports::{MembershipRepository, SourceRepository, VaultRepository};
use crate::Result;

pub struct IngestWorkflowAccess {
    sources: Arc<dyn SourceRepository>,
    vaults: Arc<dyn VaultRepository>,
    members: Arc<dyn MembershipRepository>,
    configured_sources: HashMap<Id, (SourceKind, bool)>,
    auth_required: bool,
}

impl IngestWorkflowAccess {
    pub fn new(
        sources: Arc<dyn SourceRepository>,
        vaults: Arc<dyn VaultRepository>,
        members: Arc<dyn MembershipRepository>,
        configured_sources: &[IngestionSourceDef],
        auth_required: bool,
    ) -> Self {
        Self {
            sources,
            vaults,
            members,
            configured_sources: configured_sources
                .iter()
                .map(|source| (source.id, (source.config.kind(), source.enabled)))
                .collect(),
            auth_required,
        }
    }

    pub async fn permits_owner_events(&self, vault: VaultId) -> Result<bool> {
        if self.vaults.find(vault).await?.is_none() {
            return Ok(false);
        }
        if !self.auth_required && self.vaults.instance_default().await? == vault {
            return Ok(true);
        }
        Ok(self.members.direct_owner_count(vault).await? > 0)
    }

    pub async fn permits_managed_lifecycle(
        &self,
        vault: VaultId,
        source: Id,
        kind: SourceKind,
        deleted: bool,
    ) -> Result<bool> {
        if !self.permits_owner_events(vault).await? {
            return Ok(false);
        }
        let definition = if deleted {
            self.sources.find_deleted(vault, source).await?
        } else {
            self.sources.find(vault, source).await?
        };
        Ok(definition.is_some_and(|definition| {
            definition.origin == SourceOrigin::Managed
                && definition.vault_id == Some(vault)
                && definition.config.kind() == kind
        }))
    }

    pub async fn permits(&self, vault: VaultId, source: Id, kind: SourceKind) -> Result<bool> {
        if !self.permits_owner_events(vault).await? {
            return Ok(false);
        }
        if let Some(configured) = self.configured_sources.get(&source) {
            return Ok(
                self.vaults.instance_default().await? == vault && *configured == (kind, true)
            );
        }
        Ok(self
            .sources
            .find(vault, source)
            .await?
            .is_some_and(|definition| definition.enabled && definition.config.kind() == kind))
    }
}
