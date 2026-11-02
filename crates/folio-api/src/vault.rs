//! Which vault a request without an explicit one lands in
//!
//! Routes are not vault-scoped yet, so every handler works in the caller's
//! default vault. [`resolve_default_vault`] puts that choice into the request
//! extensions once, and handlers read it through the [`DefaultVault`]
//! extractor; later route changes replace it with the vault from the path.

use std::sync::Arc;

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use folio_auth::AuthContext;
use folio_core::models::{UserId, VaultId};
use folio_core::services::VaultService;

use crate::routes::auth::{is_exempt, AuthState};

/// The vault a request works in
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultVault(pub VaultId);

/// The signed-in user behind a request, `None` when authentication is off
///
/// Recorded as `uploaded_by` on documents the request creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uploader(pub Option<UserId>);

/// State of [`resolve_default_vault`]
#[derive(Clone)]
pub struct VaultResolver {
    pub auth: AuthState,
    pub vaults: Arc<VaultService>,
}

/// Resolve the caller's default vault (creating their `Personal` vault on
/// first use) and store it in the request extensions
pub async fn resolve_default_vault(
    State(resolver): State<VaultResolver>,
    mut request: Request,
    next: Next,
) -> Response {
    if is_exempt(request.uri().path()) {
        return next.run(request).await;
    }
    let auth = &resolver.auth;
    let user = match request.extensions().get::<AuthContext>() {
        Some(ctx) => auth
            .store
            .find_user_by_id(&ctx.user_id)
            .await
            .ok()
            .flatten(),
        None => auth.current_user(request.headers()).await,
    };
    if user.is_none() && auth.required {
        return next.run(request).await;
    }
    let who = user.as_ref().map(|u| UserId(u.id.clone()));
    match resolver.vaults.default_vault(who.as_ref()).await {
        Ok(vault) => {
            request.extensions_mut().insert(DefaultVault(vault));
            request.extensions_mut().insert(Uploader(who));
            next.run(request).await
        }
        Err(e) => {
            tracing::error!("could not resolve the default vault: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for DefaultVault {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<DefaultVault>()
            .copied()
            .ok_or(StatusCode::UNAUTHORIZED)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Uploader {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Uploader>()
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)
    }
}
