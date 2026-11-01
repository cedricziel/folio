//! Persistence port for the auth domain.

use async_trait::async_trait;

use crate::model::{AccessToken, AuthCode, OAuthClient, RefreshToken, Session, User};

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("storage error: {0}")]
    Store(String),
}

#[async_trait]
pub trait AuthStore: Send + Sync {
    async fn count_users(&self) -> Result<i64, AuthError>;
    /// Creates the owner account only while no user exists. Returns `None`
    /// when one already does, so concurrent setup requests cannot both win.
    async fn create_first_user(&self, user: User) -> Result<Option<User>, AuthError>;
    async fn find_user_by_username(&self, username: &str) -> Result<Option<User>, AuthError>;
    async fn find_user_by_id(&self, id: &str) -> Result<Option<User>, AuthError>;

    async fn create_session(&self, session: Session) -> Result<(), AuthError>;
    async fn find_session(&self, id_hash: &str) -> Result<Option<Session>, AuthError>;
    async fn delete_session(&self, id_hash: &str) -> Result<(), AuthError>;

    async fn insert_client(&self, client: OAuthClient) -> Result<(), AuthError>;
    async fn find_client(&self, client_id: &str) -> Result<Option<OAuthClient>, AuthError>;
    /// Records that the client has been issued a token (keeps it from being purged).
    async fn mark_client_used(&self, client_id: &str, at: i64) -> Result<(), AuthError>;
    /// Deletes clients registered before `registered_before` that never received
    /// a token. Returns how many were removed.
    async fn purge_unused_clients(&self, registered_before: i64) -> Result<u64, AuthError>;

    async fn insert_code(&self, code: AuthCode) -> Result<(), AuthError>;
    /// Atomically removes and returns the code, making it single-use.
    async fn take_code(&self, code_hash: &str) -> Result<Option<AuthCode>, AuthError>;

    async fn insert_access_token(&self, token: AccessToken) -> Result<(), AuthError>;
    async fn find_access_token(&self, token_hash: &str) -> Result<Option<AccessToken>, AuthError>;
    async fn insert_refresh_token(&self, token: RefreshToken) -> Result<(), AuthError>;
    /// Atomically marks the refresh token as used and returns it (rotation).
    /// A token that was already used returns `None`.
    async fn take_refresh_token(&self, token_hash: &str)
        -> Result<Option<RefreshToken>, AuthError>;
    /// The grant of a refresh token that was already rotated, to detect reuse.
    async fn used_refresh_grant(&self, token_hash: &str) -> Result<Option<String>, AuthError>;
    /// Deletes every access and refresh token of a grant.
    async fn revoke_grant(&self, grant_id: &str) -> Result<(), AuthError>;
    /// Revokes the grant of an access or refresh token issued to `client_id`.
    /// Unknown tokens and tokens of other clients are ignored.
    async fn revoke_token(&self, token_hash: &str, client_id: &str) -> Result<(), AuthError>;

    /// Removes expired codes, tokens and sessions.
    async fn purge_expired(&self, now: i64) -> Result<(), AuthError>;
}
