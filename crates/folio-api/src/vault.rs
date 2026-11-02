//! Request-local vault selection and access enforcement.

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{header, request::Parts, Method, StatusCode},
    middleware::{from_fn, Next},
    response::{IntoResponse, Response},
    Router,
};
use folio_auth::AuthContext;
use folio_core::models::{UserId, VaultAction, VaultId, VaultRole};
use tower::ServiceBuilder;

use crate::error::{ApiError, ApiResult};
use crate::routes::auth::{is_exempt, AuthState};
use crate::state::AppState;
use crate::vault_context::is_scoped_path;
pub use crate::vault_context::{filters, url, VaultContext, VaultSummary, CURRENT_VAULT};

/// The authorized vault for this request, explicitly selected or the account default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultVault(pub VaultId);

/// The authenticated document uploader, or `None` when authentication is disabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uploader(pub Option<UserId>);

#[derive(Clone)]
pub struct VaultResolver {
    pub auth: AuthState,
    pub state: AppState,
}

#[derive(Clone, Copy)]
struct RequestedVault(VaultId);

/// Strip the HTML vault prefix before route matching, retaining the selected ID.
/// This keeps existing handler path extractors independent of the vault prefix.
pub fn scoped_router(router: Router) -> Router {
    Router::new().fallback_service(
        ServiceBuilder::new()
            .layer(from_fn(scope_path))
            .service(router),
    )
}

async fn scope_path(mut request: Request, next: Next) -> Response {
    if let Some(path) = request.uri().path().strip_prefix("/vaults/") {
        let (id, tail) = path.split_once('/').unwrap_or((path, ""));
        let Ok(id) = id.parse::<VaultId>() else {
            return ApiError::bad_request("Invalid vault ID").into_response();
        };
        let mut path = format!("/{tail}");
        if !is_scoped_path(&path) {
            return ApiError::not_found("Not found").into_response();
        }
        if let Some(query) = request.uri().query() {
            path.push('?');
            path.push_str(query);
        }
        let Ok(uri) = path.parse() else {
            return ApiError::bad_request("Invalid vault path").into_response();
        };
        *request.uri_mut() = uri;
        request.extensions_mut().insert(RequestedVault(id));
    }
    next.run(request).await
}

pub async fn resolve_default_vault(
    State(resolver): State<VaultResolver>,
    mut request: Request,
    next: Next,
) -> Response {
    if is_exempt(request.uri().path()) {
        return next.run(request).await;
    }
    let user = match request.extensions().get::<AuthContext>() {
        Some(ctx) => resolver
            .auth
            .store
            .find_user_by_id(&ctx.user_id)
            .await
            .ok()
            .flatten(),
        None => resolver.auth.current_user(request.headers()).await,
    };
    if user.is_none() && resolver.auth.required {
        return next.run(request).await;
    }
    let who = user.as_ref().map(|u| UserId(u.id.clone()));
    let context = match resolve(&resolver.state, &mut request, who.as_ref()).await {
        Ok(context) => context,
        Err(error) => return error.into_response(),
    };
    request.extensions_mut().insert(DefaultVault(context.id));
    request
        .extensions_mut()
        .insert(Uploader(context.user.clone()));
    let explicit = request.extensions().get::<RequestedVault>().is_some()
        || request.headers().contains_key("x-folio-vault");
    CURRENT_VAULT
        .scope(context, async move {
            let mut response = next.run(request).await;
            if let Some(location) = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .filter(|_| explicit)
            {
                let scoped = url(location);
                if let Ok(value) = scoped.parse() {
                    response.headers_mut().insert(header::LOCATION, value);
                }
            }
            response
        })
        .await
}

async fn resolve(
    state: &AppState,
    request: &mut Request,
    user: Option<&UserId>,
) -> ApiResult<VaultContext> {
    let default = state.vaults.default_vault(user).await?;
    let header = request
        .headers()
        .get("x-folio-vault")
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|s| s.parse::<VaultId>().ok())
                .ok_or_else(|| ApiError::bad_request("Invalid vault ID"))
        })
        .transpose()?;
    let api_vault = request
        .uri()
        .path()
        .strip_prefix("/api/vaults/")
        .map(|path| {
            path.split('/')
                .next()
                .unwrap_or("")
                .parse::<VaultId>()
                .map_err(|_| ApiError::bad_request("Invalid vault ID"))
        })
        .transpose()?;
    let path = request
        .extensions()
        .get::<RequestedVault>()
        .map(|v| v.0)
        .or(api_vault);
    if matches!((path, header), (Some(a), Some(b)) if a != b) {
        return Err(ApiError::bad_request("Conflicting vault selections"));
    }
    let id = path.or(header).unwrap_or(default);
    let role = match user {
        Some(user) => state.access.effective_role(user, id).await?,
        None if id == default => Some(VaultRole::Owner),
        None => None,
    }
    .ok_or_else(|| ApiError::not_found("Vault not found"))?;
    if let Some(user) = user {
        let action = action(state, request, id).await?;
        if !role.permits(&action, user) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "This vault role does not allow that action",
            ));
        }
    }
    let vaults = state
        .vaults
        .list(user)
        .await?
        .into_iter()
        .map(|(v, role)| VaultSummary {
            id: v.id,
            name: v.name,
            role,
        })
        .collect();
    Ok(VaultContext {
        id,
        role,
        vaults,
        user: user.cloned(),
    })
}

async fn action(state: &AppState, request: &mut Request, vault: VaultId) -> ApiResult<VaultAction> {
    let path = request.uri().path();
    let path = path.strip_prefix("/api").unwrap_or(path);
    let mutation = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if path == "/workflows"
        || path.starts_with("/workflows/")
        || path == "/sources"
        || path.starts_with("/sources/")
        || path.starts_with("/settings/sources")
    {
        return Ok(VaultAction::ManageWorkflows);
    }
    if !mutation || path == "/documents/search" {
        return Ok(VaultAction::Read);
    }
    if matches!(
        path,
        "/documents" | "/documents/" | "/documents/upload" | "/share"
    ) {
        return Ok(VaultAction::CreateDocument);
    }
    if let Some(tail) = path.strip_prefix("/documents/") {
        let (id, operation) = tail.split_once('/').unwrap_or((tail, ""));
        let id = id
            .parse()
            .map_err(|_| ApiError::not_found("Document not found"))?;
        let lifecycle = (request.method() == Method::DELETE && matches!(operation, "" | "purge"))
            || (request.method() == Method::POST
                && matches!(operation, "delete" | "restore" | "delete-forever"));
        let document = state.document_repo.find_by_id(vault, id).await?;
        let document = match document {
            Some(document) => Some(document),
            None if lifecycle => state.document_repo.find_trashed_by_id(vault, id).await?,
            None => None,
        }
        .ok_or_else(|| ApiError::not_found("Document not found"))?;
        return Ok(if lifecycle {
            VaultAction::DeleteDocument {
                uploaded_by: document.uploaded_by,
            }
        } else {
            VaultAction::EditDocument {
                uploaded_by: document.uploaded_by,
            }
        });
    }
    if ["/tags", "/correspondents", "/document_types", "/manage"]
        .iter()
        .any(|prefix| {
            path == *prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|tail| tail.starts_with('/'))
        })
    {
        return Ok(VaultAction::ManageVocabulary);
    }
    Ok(VaultAction::Read)
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
