//! Rules every Paperless source operation follows, for the API and MCP alike

use crate::error::{Error, Result};
use crate::models::{Caller, Id, IngestionSourceDef, VaultId, VaultRole};
use crate::ports::SourceRepository;

/// Why a second live Paperless source in a vault is refused
pub const PAPERLESS_SLOT_TAKEN: &str = "This vault already has a Paperless source";

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
    let is_other_paperless =
        |def: &IngestionSourceDef| def.config.is_paperless() && Some(def.id) != except;
    if configured
        .iter()
        .any(|def| def.vault_id == Some(vault) && is_other_paperless(def))
    {
        return Err(Error::Conflict(
            "This vault's Paperless source is defined in the server configuration".into(),
        ));
    }
    if managed.list(vault).await?.iter().any(is_other_paperless) {
        return Err(Error::Conflict(PAPERLESS_SLOT_TAKEN.into()));
    }
    Ok(())
}

/// Splits managed sources into the ones to run and the Paperless ones that a
/// config-defined Paperless source of the same vault overrides
pub fn split_shadowed_paperless(
    configured: &[IngestionSourceDef],
    managed: Vec<IngestionSourceDef>,
) -> (Vec<IngestionSourceDef>, Vec<IngestionSourceDef>) {
    let config_vaults: Vec<_> = configured
        .iter()
        .filter(|def| def.config.is_paperless())
        .filter_map(|def| def.vault_id)
        .collect();
    managed.into_iter().partition(|def| {
        !(def.config.is_paperless()
            && def
                .vault_id
                .is_some_and(|vault| config_vaults.contains(&vault)))
    })
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

    fn source(kind: &str, vault: VaultId) -> IngestionSourceDef {
        let config = match kind {
            "paperless" => {
                serde_json::json!({"type": "paperless", "url": "https://p.example", "token": "t"})
            }
            _ => serde_json::json!({"type": "folder", "path": "/scans"}),
        };
        IngestionSourceDef {
            lifecycle: None,
            id: Id::new_v4(),
            name: kind.into(),
            enabled: true,
            origin: crate::models::SourceOrigin::Managed,
            config: serde_json::from_value(config).unwrap(),
            vault_id: Some(vault),
        }
    }

    #[test]
    fn a_config_paperless_source_shadows_the_managed_one_of_its_vault_only() {
        let (home, other) = (VaultId::new(), VaultId::new());
        let configured = vec![source("paperless", home)];
        let shadowed = source("paperless", home);
        let kept = vec![source("folder", home), source("paperless", other)];
        let mut managed = kept.clone();
        managed.push(shadowed.clone());

        let (run, skipped) = split_shadowed_paperless(&configured, managed);

        assert_eq!(run, kept);
        assert_eq!(skipped, vec![shadowed]);
    }
}
