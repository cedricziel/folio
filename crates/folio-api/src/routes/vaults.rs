use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::vault::{Uploader, VaultSummary};
use folio_core::models::{
    AuditEvent, GroupId, Principal, UserId, Vault, VaultId, VaultMember, VaultRole,
};

#[derive(Serialize, ToSchema)]
pub struct VaultDiscovery {
    pub vaults: Vec<VaultSummary>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub default_vault_id: Option<VaultId>,
    pub deleted_vaults: Vec<Vault>,
}

#[derive(Default, Deserialize)]
pub struct DiscoveryQuery {
    #[serde(default)]
    include_deleted: bool,
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct VaultNameInput {
    pub name: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    User,
    Group,
}

impl PrincipalKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Group => "group",
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MembershipInput {
    pub principal_type: PrincipalKind,
    pub principal_id: String,
    #[schema(value_type = String)]
    pub role: VaultRole,
}

#[derive(Serialize, ToSchema)]
pub struct MemberView {
    #[schema(value_type = uuid::Uuid)]
    pub vault_id: VaultId,
    pub principal_type: PrincipalKind,
    pub principal_id: String,
    pub role: String,
    pub granted_by: Option<String>,
    pub granted_at: DateTime<Utc>,
}

impl From<VaultMember> for MemberView {
    fn from(member: VaultMember) -> Self {
        let (principal_type, principal_id) = match member.principal {
            Principal::User(user) => (PrincipalKind::User, user.0),
            Principal::Group(group) => (PrincipalKind::Group, group.0),
        };
        Self {
            vault_id: member.vault_id,
            principal_type,
            principal_id,
            role: member.role.as_str().into(),
            granted_by: member.granted_by.map(|user| user.0),
            granted_at: member.granted_at,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct VaultHistory {
    pub id: uuid::Uuid,
    pub at: DateTime<Utc>,
    pub actor: Option<String>,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub details: serde_json::Value,
}

impl From<AuditEvent> for VaultHistory {
    fn from(event: AuditEvent) -> Self {
        Self {
            id: event.id,
            at: event.at,
            actor: event.actor.map(|user| user.0),
            action: event.action,
            target_type: event.target_type,
            target_id: event.target_id,
            details: event.details,
        }
    }
}

impl VaultHistory {
    pub(super) fn description(&self) -> String {
        let name = self.details["name"].as_str().unwrap_or("Vault");
        let principal = self.details["principal_id"].as_str().unwrap_or("Member");
        let role = self.details["role"].as_str().unwrap_or("access");
        match self.action.as_str() {
            "vault.created" => format!("Created {name}"),
            "vault.updated" => format!("Renamed to {name}"),
            "vault.deleted" => format!("Deleted {name}"),
            "vault.restored" => format!("Restored {name}"),
            "vault_membership.granted" => format!("Granted {role} access to {principal}"),
            "vault_membership.role_changed" => format!("Changed {principal} to {role}"),
            "vault_membership.revoked" => format!("Revoked access for {principal}"),
            "document.moved" => "Moved a document".into(),
            _ => "Vault activity".into(),
        }
    }
}

pub(super) fn signed_in(user: Option<UserId>) -> ApiResult<UserId> {
    user.ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Sign in to manage vaults"))
}

pub(super) fn principal(kind: PrincipalKind, id: String) -> ApiResult<Principal> {
    if id.trim().is_empty() || id.len() > 512 {
        return Err(ApiError::bad_request("A user or group ID is required"));
    }
    Ok(match kind {
        PrincipalKind::User => Principal::User(UserId(id)),
        PrincipalKind::Group => Principal::Group(GroupId(id)),
    })
}

pub(super) async fn owner(
    state: &AppState,
    user: &UserId,
    vault: VaultId,
    retained: bool,
) -> ApiResult<()> {
    match state.access.effective_role(user, vault).await? {
        Some(VaultRole::Owner) => Ok(()),
        Some(_) => Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Only a vault owner can manage this vault",
        )),
        None if retained
            && state
                .vaults
                .retained_for_owner(user, vault)
                .await?
                .is_some() =>
        {
            Ok(())
        }
        None => Err(ApiError::not_found("Vault not found")),
    }
}

#[utoipa::path(get, path = "/api/vaults", operation_id = "listVaults", tag = "vaults",
    params(("include_deleted" = Option<bool>, Query, description = "Include deleted vault metadata owned by the current user")),
    responses((status = 200, description = "Accessible vaults and account default", body = VaultDiscovery)))]
pub(crate) async fn list_vaults(
    Uploader(user): Uploader,
    Query(query): Query<DiscoveryQuery>,
    State(state): State<AppState>,
) -> ApiResult<Json<VaultDiscovery>> {
    let vaults: Vec<VaultSummary> = state
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
    let deleted_vaults = if query.include_deleted {
        state.vaults.deleted(&signed_in(user.clone())?).await?
    } else {
        Vec::new()
    };
    let default_vault_id = if vaults.is_empty() {
        None
    } else {
        Some(state.vaults.default_vault(user.as_ref()).await?)
    };
    Ok(Json(VaultDiscovery {
        vaults,
        default_vault_id,
        deleted_vaults,
    }))
}

#[utoipa::path(post, path = "/api/vaults", operation_id = "createVault", tag = "vaults", request_body = VaultNameInput,
    responses((status = 201, description = "Vault and initial owner created atomically", body = Vault)))]
pub(crate) async fn create_vault(
    Uploader(user): Uploader,
    State(state): State<AppState>,
    Json(input): Json<VaultNameInput>,
) -> ApiResult<(StatusCode, Json<Vault>)> {
    let user = signed_in(user)?;
    Ok((
        StatusCode::CREATED,
        Json(state.vaults.create(&user, &input.name).await?),
    ))
}

#[utoipa::path(get, path = "/api/vaults/{vault_id}", operation_id = "getVault", tag = "vaults", params(("vault_id" = uuid::Uuid, Path)),
    responses((status = 200, description = "Vault lifecycle metadata", body = Vault)))]
pub(crate) async fn get_vault(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
) -> ApiResult<Json<Vault>> {
    let user = signed_in(user)?;
    let vault = match state.vaults.find(id).await? {
        Some(vault) => vault,
        None => state
            .vaults
            .retained_for_owner(&user, id)
            .await?
            .ok_or_else(|| ApiError::not_found("Vault not found"))?,
    };
    Ok(Json(vault))
}

#[utoipa::path(put, path = "/api/vaults/{vault_id}", operation_id = "renameVault", tag = "vaults", request_body = VaultNameInput, params(("vault_id" = uuid::Uuid, Path)),
    responses((status = 200, description = "Renamed vault", body = Vault)))]
pub(crate) async fn rename_vault(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
    Json(input): Json<VaultNameInput>,
) -> ApiResult<Json<Vault>> {
    let user = signed_in(user)?;
    owner(&state, &user, id, false).await?;
    Ok(Json(state.vaults.rename(&user, id, &input.name).await?))
}

#[utoipa::path(delete, path = "/api/vaults/{vault_id}", operation_id = "deleteVault", tag = "vaults", params(("vault_id" = uuid::Uuid, Path)),
    responses((status = 204, description = "Vault retained and hidden"), (status = 409, description = "Select another default first")))]
pub(crate) async fn delete_vault(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
) -> ApiResult<StatusCode> {
    let user = signed_in(user)?;
    owner(&state, &user, id, true).await?;
    state.vaults.delete(&user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/vaults/{vault_id}/restore", operation_id = "restoreVault", tag = "vaults", params(("vault_id" = uuid::Uuid, Path)),
    responses((status = 200, description = "Restored without replaying historical work", body = Vault)))]
pub(crate) async fn restore_vault(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
) -> ApiResult<Json<Vault>> {
    let user = signed_in(user)?;
    owner(&state, &user, id, true).await?;
    Ok(Json(state.vaults.restore(&user, id).await?))
}

#[utoipa::path(post, path = "/api/vaults/{vault_id}/default", operation_id = "setDefaultVault", tag = "vaults", params(("vault_id" = uuid::Uuid, Path)),
    responses((status = 204, description = "Account default selected")))]
pub(crate) async fn set_default_vault(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
) -> ApiResult<StatusCode> {
    state.vaults.set_default(&signed_in(user)?, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/vaults/{vault_id}/members", operation_id = "listVaultMembers", tag = "vaults", params(("vault_id" = uuid::Uuid, Path)),
    responses((status = 200, description = "Current vault grants", body = Vec<MemberView>)))]
pub(crate) async fn list_members(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<MemberView>>> {
    Ok(Json(
        state
            .vaults
            .members(&signed_in(user)?, id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

#[utoipa::path(post, path = "/api/vaults/{vault_id}/members", operation_id = "grantVaultMember", tag = "vaults", params(("vault_id" = uuid::Uuid, Path)), request_body = MembershipInput,
    responses((status = 204, description = "Granted or changed a role"), (status = 409, description = "The last direct owner is protected")))]
pub(crate) async fn grant_member(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
    Json(input): Json<MembershipInput>,
) -> ApiResult<StatusCode> {
    let user = signed_in(user)?;
    owner(&state, &user, id, false).await?;
    state
        .vaults
        .grant(
            &user,
            id,
            &principal(input.principal_type, input.principal_id)?,
            input.role,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/vaults/{vault_id}/members/{principal_type}/{principal_id}", operation_id = "revokeVaultMember", tag = "vaults", params(("vault_id" = uuid::Uuid, Path), ("principal_type" = PrincipalKind, Path), ("principal_id" = String, Path)),
    responses((status = 204, description = "Revoked a grant"), (status = 409, description = "The last direct owner is protected")))]
pub(crate) async fn revoke_member(
    Path((id, kind, principal_id)): Path<(VaultId, PrincipalKind, String)>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
) -> ApiResult<StatusCode> {
    let user = signed_in(user)?;
    owner(&state, &user, id, false).await?;
    state
        .vaults
        .revoke(&user, id, &principal(kind, principal_id)?)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/vaults/{vault_id}/history", operation_id = "listVaultHistory", tag = "vaults", params(("vault_id" = uuid::Uuid, Path)),
    responses((status = 200, description = "Owner-visible retained lifecycle history", body = Vec<VaultHistory>)))]
pub(crate) async fn list_history(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
) -> ApiResult<Json<Vec<VaultHistory>>> {
    let user = signed_in(user)?;
    owner(&state, &user, id, true).await?;
    Ok(Json(
        state
            .vaults
            .history(&user, id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
