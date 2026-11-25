use std::sync::Arc;

use crate::models::VaultId;
use crate::ports::{MembershipRepository, VaultRepository};
use crate::Result;

pub struct VaultWorkflowAccess {
    vaults: Arc<dyn VaultRepository>,
    members: Arc<dyn MembershipRepository>,
}

impl VaultWorkflowAccess {
    pub fn new(vaults: Arc<dyn VaultRepository>, members: Arc<dyn MembershipRepository>) -> Self {
        Self { vaults, members }
    }

    pub async fn permits(&self, vault: VaultId) -> Result<bool> {
        Ok(self.vaults.find(vault).await?.is_some()
            && self.members.direct_owner_count(vault).await? > 0)
    }
}
