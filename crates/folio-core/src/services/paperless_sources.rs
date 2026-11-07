//! Rules every Paperless source operation follows, for the API and MCP alike

use crate::error::{Error, Result};
use crate::models::{Caller, Id, IngestionSourceDef, SourceConfig, VaultId, VaultRole};
use crate::ports::SourceRepository;

/// A Paperless source reads a whole library with a long-lived token, so only
/// an instance admin who owns the vault may manage it
pub fn authorize_paperless(caller: &Caller, vault_role: Option<VaultRole>) -> Result<()> {
    if caller.is_instance_admin() && vault_role == Some(VaultRole::Owner) {
        Ok(())
    } else {
        Err(Error::Forbidden(
            "Only an instance admin who owns this vault can manage its Paperless source".into(),
        ))
    }
}

/// A vault has at most one live Paperless source; one from the configuration wins
///
/// `except` is the source being updated or restored, which may keep its own slot.
pub async fn ensure_paperless_slot(
    managed: &dyn SourceRepository,
    configured: &[IngestionSourceDef],
    vault: VaultId,
    except: Option<Id>,
) -> Result<()> {
    let is_other_paperless = |def: &IngestionSourceDef| {
        matches!(def.config, SourceConfig::Paperless(_)) && Some(def.id) != except
    };
    if configured
        .iter()
        .any(|def| def.vault_id == Some(vault) && is_other_paperless(def))
    {
        return Err(Error::Conflict(
            "This vault's Paperless source is defined in the server configuration".into(),
        ));
    }
    if managed.list(vault).await?.iter().any(is_other_paperless) {
        return Err(Error::Conflict(
            "This vault already has a Paperless source".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::UserId;

    fn caller(is_admin: bool, is_owner: bool) -> Caller {
        Caller {
            user_id: Some(UserId::from("ada")),
            is_admin,
            is_owner,
        }
    }

    #[test]
    fn only_instance_admins_who_own_the_vault_pass() {
        for (caller, role, allowed) in [
            (caller(true, false), Some(VaultRole::Owner), true),
            (caller(false, true), Some(VaultRole::Owner), true),
            (caller(false, false), Some(VaultRole::Owner), false),
            (caller(true, true), Some(VaultRole::Editor), false),
            (caller(true, false), Some(VaultRole::Viewer), false),
            (caller(true, true), None, false),
        ] {
            let result = authorize_paperless(&caller, role);
            assert_eq!(result.is_ok(), allowed, "{caller:?} {role:?}");
            if let Err(error) = result {
                assert!(matches!(error, Error::Forbidden(_)), "{error:?}");
            }
        }
    }
}
