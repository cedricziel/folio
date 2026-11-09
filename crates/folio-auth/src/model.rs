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
    /// Zero for sessions created before this was recorded.
    pub created_at: i64,
    pub user_agent: Option<String>,
}

/// A user's TOTP enrollment. The seed is encrypted; it counts only once
/// `confirmed` is set by entering a valid code.
#[derive(Debug, Clone)]
pub struct TotpRecord {
    pub user_id: String,
    pub secret_enc: String,
    pub confirmed: bool,
    /// The newest step already used, so a code cannot be replayed.
    pub last_step: i64,
}

/// A registered passkey. `data` is the serialised WebAuthn credential.
#[derive(Debug, Clone)]
pub struct PasskeyRecord {
    pub id: String,
    pub user_id: String,
    pub name: String,
    /// Base64url credential id.
    pub credential_id: String,
    pub data: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

/// An OAuth client that holds live tokens of a user.
#[derive(Debug, Clone)]
pub struct ConnectedClient {
    pub client_id: String,
    pub client_name: String,
    /// When the newest of its tokens stops working.
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

/// A user-managed credential. The full secret is never stored.
#[derive(Clone)]
pub struct PersonalToken {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub token_hash: String,
    pub prefix: String,
    pub audience: String,
    pub scopes: Vec<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
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
    pub personal_token_id: Option<String>,
    pub user_id: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub audience: String,
    /// The OAuth grant the access token belongs to; empty for personal tokens
    /// and tokens issued before grants existed
    pub grant_id: String,
}

impl AuthContext {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}
