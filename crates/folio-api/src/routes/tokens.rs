use axum::{
    extract::{rejection::JsonRejection, Path, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get},
    Json, Router,
};
use folio_auth::{
    crypto::{hash_token, random_token},
    server::now,
    PersonalToken, SCOPES,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::auth::AuthState;

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePersonalTokenRequest {
    pub name: String,
    #[schema(pattern = "^(api|mcp)$")]
    pub audience: String,
    pub scopes: Vec<String>,
    #[schema(minimum = 1, maximum = 365)]
    pub expires_in_days: u16,
}

#[derive(Serialize, ToSchema)]
pub struct PersonalTokenMetadata {
    pub id: String,
    pub name: String,
    pub prefix: String,
    #[schema(pattern = "^(api|mcp)$")]
    pub audience: String,
    pub scopes: Vec<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
}

impl From<PersonalToken> for PersonalTokenMetadata {
    fn from(token: PersonalToken) -> Self {
        Self {
            id: token.id,
            name: token.name,
            prefix: token.prefix,
            audience: if token.audience.ends_with("/mcp") {
                "mcp"
            } else {
                "api"
            }
            .into(),
            scopes: token.scopes,
            created_at: token.created_at,
            expires_at: token.expires_at,
            revoked_at: token.revoked_at,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct CreatedPersonalToken {
    #[serde(flatten)]
    pub metadata: PersonalTokenMetadata,
    pub token: String,
}

pub fn router<S>(auth: AuthState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/api/auth/tokens",
            get(list_personal_tokens).post(create_personal_token),
        )
        .route("/api/auth/tokens/{id}", delete(revoke_personal_token))
        .route_layer(middleware::from_fn_with_state(auth.clone(), session_only))
        .with_state(auth)
}

async fn session_only(State(auth): State<AuthState>, request: Request, next: Next) -> Response {
    let mut response = if request.headers().contains_key(header::AUTHORIZATION) {
        error(StatusCode::UNAUTHORIZED, "session_required")
    } else if auth.session(request.headers()).await.is_none() {
        error(StatusCode::UNAUTHORIZED, "unauthorized")
    } else if request.method() != axum::http::Method::GET
        && request.method() != axum::http::Method::HEAD
        && !auth.is_same_origin(request.headers())
    {
        error(StatusCode::FORBIDDEN, "cross_origin_request")
    } else {
        next.run(request).await
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

fn error(status: StatusCode, code: &str) -> Response {
    (status, Json(serde_json::json!({"error": code}))).into_response()
}

#[utoipa::path(get, path = "/api/auth/tokens", operation_id = "listPersonalTokens", tag = "authentication",
    security(("sessionAuth" = [])),
    responses((status = 200, description = "Owned token metadata, including revoked and expired credentials. Secrets are never returned. Cache-Control: no-store.", body = Vec<PersonalTokenMetadata>),
        (status = 401, description = "A signed-in account session is required; bearer credentials are rejected even with a session cookie"),
        (status = 403, description = "Cross-origin mutation rejected"),
        (status = 500, description = "Credential storage failed")))]
pub(crate) async fn list_personal_tokens(
    State(auth): State<AuthState>,
    headers: HeaderMap,
) -> Response {
    let Some(user) = auth.current_user(&headers).await else {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    match auth.store.list_personal_tokens(&user.id).await {
        Ok(tokens) => Json(
            tokens
                .into_iter()
                .map(PersonalTokenMetadata::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "storage_error"),
    }
}

#[utoipa::path(post, path = "/api/auth/tokens", operation_id = "createPersonalToken", tag = "authentication",
    security(("sessionAuth" = [])), request_body = CreatePersonalTokenRequest,
    responses((status = 201, description = "Token secret shown once. Audience is api or mcp, scopes are explicit documents:read and/or documents:write, expiry is 1–365 days. Cache-Control: no-store.", body = CreatedPersonalToken),
        (status = 400, description = "Invalid name, audience, scopes, expiry, or JSON"),
        (status = 401, description = "A signed-in account session is required; bearer credentials are rejected even with a session cookie"),
        (status = 403, description = "Cross-origin mutation rejected"),
        (status = 500, description = "Credential storage failed")))]
pub(crate) async fn create_personal_token(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    body: Result<Json<CreatePersonalTokenRequest>, JsonRejection>,
) -> Response {
    let Some(user) = auth.current_user(&headers).await else {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let input = match body {
        Ok(Json(input)) => input,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let name = input.name.trim();
    if name.is_empty()
        || name.chars().count() > 64
        || !matches!(input.audience.as_str(), "api" | "mcp")
        || !(1..=365).contains(&input.expires_in_days)
        || input.scopes.is_empty()
        || input.scopes.len() > SCOPES.len()
        || input
            .scopes
            .iter()
            .any(|scope| !SCOPES.contains(&scope.as_str()))
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let mut scopes = input.scopes;
    scopes.sort();
    scopes.dedup();
    let token = format!("folio_pat_{}", random_token());
    let at = now();
    let record = PersonalToken {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user.id,
        name: name.into(),
        token_hash: hash_token(&token),
        prefix: token[..18].into(),
        audience: format!("{}/{}", auth.base(), input.audience),
        scopes,
        created_at: at,
        expires_at: at + i64::from(input.expires_in_days) * 86_400,
        revoked_at: None,
    };
    match auth.store.insert_personal_token(record.clone()).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(CreatedPersonalToken {
                metadata: record.into(),
                token,
            }),
        )
            .into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "storage_error"),
    }
}

#[utoipa::path(delete, path = "/api/auth/tokens/{id}", operation_id = "revokePersonalToken", tag = "authentication",
    security(("sessionAuth" = [])), params(("id" = String, Path, description = "Owned credential ID")),
    responses((status = 204, description = "Credential revoked immediately, including subsequent MCP requests. Cache-Control: no-store."),
        (status = 401, description = "A signed-in account session is required; bearer credentials are rejected even with a session cookie"),
        (status = 403, description = "Cross-origin mutation rejected"),
        (status = 404, description = "Credential does not exist or belongs to another account"),
        (status = 500, description = "Credential storage failed")))]
pub(crate) async fn revoke_personal_token(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let Some(user) = auth.current_user(&headers).await else {
        return error(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    match auth.store.revoke_personal_token(&user.id, &id, now()).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "storage_error"),
    }
}
