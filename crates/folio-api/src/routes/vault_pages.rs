use askama::Template;
use axum::{
    extract::{Path, State},
    response::{Html, Redirect},
    routing::{get, post},
    Form, Router,
};
use serde::Deserialize;

use super::vaults::{owner, principal, signed_in, MemberView, PrincipalKind, VaultHistory};
use super::view::{Nav, Sidebar};
use crate::vault::filters;
use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
    vault::{Uploader, CURRENT_VAULT},
};
use folio_core::models::{Vault, VaultId, VaultRole};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings/vaults", get(page))
        .route("/settings/vaults/create", post(create))
        .route("/settings/vaults/{id}/rename", post(rename))
        .route("/settings/vaults/{id}/delete", post(delete))
        .route("/settings/vaults/{id}/restore", post(restore))
        .route("/settings/vaults/{id}/default", post(set_default))
        .route("/settings/vaults/{id}/members", post(grant))
        .route("/settings/vaults/{id}/members/revoke", post(revoke))
}

struct ActiveVault {
    vault: Vault,
    role: String,
    owner: bool,
    default: bool,
    members: Vec<MemberView>,
    history: Vec<VaultHistory>,
}

struct DeletedVault {
    vault: Vault,
    history: Vec<VaultHistory>,
}

#[derive(Template)]
#[template(path = "vaults.html")]
struct VaultsTemplate {
    title: String,
    sidebar: Sidebar,
    vaults: Vec<ActiveVault>,
    deleted: Vec<DeletedVault>,
}

async fn page(Uploader(user): Uploader, State(state): State<AppState>) -> ApiResult<Html<String>> {
    let user = signed_in(user)?;
    let active = state.vaults.list(Some(&user)).await?;
    let default = if active.is_empty() {
        None
    } else {
        Some(state.vaults.default_vault(Some(&user)).await?)
    };
    let mut vaults = Vec::new();
    for (vault, role) in active {
        let owner = role == VaultRole::Owner;
        let members = if owner {
            state
                .vaults
                .members(&user, vault.id)
                .await?
                .into_iter()
                .map(Into::into)
                .collect()
        } else {
            Vec::new()
        };
        let history = if owner {
            state
                .vaults
                .history(&user, vault.id)
                .await?
                .into_iter()
                .map(Into::into)
                .collect()
        } else {
            Vec::new()
        };
        vaults.push(ActiveVault {
            default: Some(vault.id) == default,
            vault,
            role: role.as_str().into(),
            owner,
            members,
            history,
        });
    }
    let mut deleted = Vec::new();
    for vault in state.vaults.deleted(&user).await? {
        let history = state
            .vaults
            .history(&user, vault.id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        deleted.push(DeletedVault { vault, history });
    }
    let template = VaultsTemplate {
        title: "Vaults - Folio".into(),
        sidebar: match CURRENT_VAULT.try_with(|context| context.id).ok() {
            Some(current) => Sidebar::load(&state, current, Nav::Settings).await?,
            None => Sidebar::without_vault(Nav::Settings),
        },
        vaults,
        deleted,
    };
    Ok(Html(
        template
            .render()
            .map_err(|error| ApiError::internal(error.to_string()))?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameForm {
    name: String,
}

async fn create(
    Uploader(user): Uploader,
    State(state): State<AppState>,
    Form(input): Form<NameForm>,
) -> ApiResult<Redirect> {
    state.vaults.create(&signed_in(user)?, &input.name).await?;
    Ok(Redirect::to("/settings/vaults"))
}

async fn rename(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
    Form(input): Form<NameForm>,
) -> ApiResult<Redirect> {
    let user = signed_in(user)?;
    owner(&state, &user, id, false).await?;
    state.vaults.rename(&user, id, &input.name).await?;
    Ok(Redirect::to("/settings/vaults"))
}

async fn delete(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
) -> ApiResult<Redirect> {
    let user = signed_in(user)?;
    owner(&state, &user, id, true).await?;
    state.vaults.delete(&user, id).await?;
    Ok(Redirect::to("/settings/vaults"))
}

async fn restore(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
) -> ApiResult<Redirect> {
    let user = signed_in(user)?;
    owner(&state, &user, id, true).await?;
    state.vaults.restore(&user, id).await?;
    Ok(Redirect::to("/settings/vaults"))
}

async fn set_default(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
) -> ApiResult<Redirect> {
    state.vaults.set_default(&signed_in(user)?, id).await?;
    Ok(Redirect::to("/settings/vaults"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberForm {
    principal_type: PrincipalKind,
    principal_id: String,
    role: VaultRole,
}

async fn grant(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
    Form(input): Form<MemberForm>,
) -> ApiResult<Redirect> {
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
    Ok(Redirect::to("/settings/vaults"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeForm {
    principal_type: PrincipalKind,
    principal_id: String,
}

async fn revoke(
    Path(id): Path<VaultId>,
    Uploader(user): Uploader,
    State(state): State<AppState>,
    Form(input): Form<RevokeForm>,
) -> ApiResult<Redirect> {
    let user = signed_in(user)?;
    owner(&state, &user, id, false).await?;
    state
        .vaults
        .revoke(
            &user,
            id,
            &principal(input.principal_type, input.principal_id)?,
        )
        .await?;
    Ok(Redirect::to("/settings/vaults"))
}
