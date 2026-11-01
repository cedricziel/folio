//! Domain types. All timestamps are unix seconds.

use serde::{Deserialize, Serialize};

/// Scopes the authorization server can grant.
pub const SCOPES: [&str; 2] = ["documents:read", "documents:write"];

#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub created_at: i64,
}

/// A login session. Only the hash of the cookie value is stored.
#[derive(Debug, Clone)]
pub struct Session {
    pub id_hash: String,
    pub user_id: String,
    pub expires_at: i64,
}

/// A public client registered through RFC 7591 dynamic client registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct AuthCode {
    pub code_hash: String,
    pub client_id: String,
    pub user_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub scopes: Vec<String>,
    pub audience: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token_hash: String,
    pub client_id: String,
    pub user_id: String,
    pub scopes: Vec<String>,
    pub audience: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct RefreshToken {
    pub token_hash: String,
    pub client_id: String,
    pub user_id: String,
    pub scopes: Vec<String>,
    pub audience: String,
    pub expires_at: i64,
}

/// The authenticated caller, inserted into request extensions by
/// [`crate::require_bearer`].
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub audience: String,
}

impl AuthContext {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}
