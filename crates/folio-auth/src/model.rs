//! Domain types. All timestamps are unix seconds.

use serde::{Deserialize, Serialize};

/// Scopes the authorization server can grant.
pub const SCOPES: [&str; 2] = ["documents:read", "documents:write"];

#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub username: String,
    /// Argon2 hash, or [`NO_PASSWORD`] for users who sign in through an
    /// external identity provider.
    pub password_hash: String,
    pub created_at: i64,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub is_admin: bool,
    /// Set once, atomically, for the very first account. The owner is always
    /// admin and is never demoted by an external group or role mapping.
    pub is_owner: bool,
    /// Where the account comes from: [`SOURCE_LOCAL`], [`SOURCE_FORWARD`], ...
    pub auth_source: String,
    /// The identifier at the external provider; `None` for local accounts.
    pub external_id: Option<String>,
}

/// Marker stored instead of a hash; never parses as a password hash.
pub const NO_PASSWORD: &str = "!";
pub const SOURCE_LOCAL: &str = "local";
pub const SOURCE_FORWARD: &str = "forward";
/// Prefix of `auth_source` for OpenID Connect users; the issuer follows.
pub const SOURCE_OIDC_PREFIX: &str = "oidc:";

/// Groups live in two namespaces so an IdP group `admins` never matches a
/// local group `admins`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupSource {
    Local,
    External,
}

impl GroupSource {
    pub fn as_str(self) -> &'static str {
        match self {
            GroupSource::Local => "local",
            GroupSource::External => "external",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "local" => Some(GroupSource::Local),
            "external" => Some(GroupSource::External),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Group {
    pub source: GroupSource,
    pub name: String,
}

/// A login session. Only the hash of the cookie value is stored.
#[derive(Debug, Clone)]
pub struct Session {
    pub id_hash: String,
    pub user_id: String,
    pub expires_at: i64,
}

/// An OpenID Connect authorization request in flight. Only hashes of the
/// state and of the browser-binding token are stored.
#[derive(Debug, Clone)]
pub struct OidcLogin {
    pub state_hash: String,
    pub browser_hash: String,
    pub nonce: String,
    pub pkce_verifier: String,
    /// Where to send the user after login, already sanitised.
    pub next: String,
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
    /// Groups the code and every token derived from it.
    pub grant_id: String,
}

#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token_hash: String,
    pub client_id: String,
    pub user_id: String,
    pub scopes: Vec<String>,
    pub audience: String,
    pub expires_at: i64,
    /// Groups the code and every token derived from it.
    pub grant_id: String,
}

#[derive(Debug, Clone)]
pub struct RefreshToken {
    pub token_hash: String,
    pub client_id: String,
    pub user_id: String,
    pub scopes: Vec<String>,
    pub audience: String,
    pub expires_at: i64,
    /// Groups the code and every token derived from it.
    pub grant_id: String,
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
