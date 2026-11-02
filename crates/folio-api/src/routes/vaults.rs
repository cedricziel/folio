use axum::{extract::State, Json};
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::ApiResult;
use crate::state::AppState;
use crate::vault::{Uploader, VaultSummary};
use folio_core::models::VaultId;

#[derive(Serialize, ToSchema)]
pub struct VaultDiscovery {
    pub vaults: Vec<VaultSummary>,
    #[schema(value_type = uuid::Uuid)]
    pub default_vault_id: VaultId,
}

#[utoipa::path(get, path = "/api/vaults", operation_id = "listVaults", tag = "vaults",
    responses((status = 200, description = "Accessible vaults and the account default", body = VaultDiscovery))
)]
pub(crate) async fn list_vaults(
    Uploader(user): Uploader,
    State(state): State<AppState>,
) -> ApiResult<Json<VaultDiscovery>> {
    let vaults = state
        .vaults
        .list(user.as_ref())
        .await?
        .into_iter()
        .map(|(v, role)| VaultSummary {
            id: v.id,
            name: v.name,
            role,
        })
        .collect();
    let default_vault_id = state.vaults.default_vault(user.as_ref()).await?;
    Ok(Json(VaultDiscovery {
        vaults,
        default_vault_id,
    }))
}
