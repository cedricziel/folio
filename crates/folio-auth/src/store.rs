//! Persistence port for the auth domain.

use async_trait::async_trait;

use crate::model::{
    AccessToken, AuthCode, Group, OAuthClient, OidcLogin, RefreshToken, Session, User,
};

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("storage error: {0}")]
    Store(String),
    #[error("already exists")]
    Conflict,
}

#[async_trait]
pub trait AuthStore: Send + Sync {
    async fn count_users(&self) -> Result<i64, AuthError>;
    /// Creates the owner account only while no user exists. Returns `None`
    /// when one already does, so concurrent setup requests cannot both win.
    async fn create_first_user(&self, user: User) -> Result<Option<User>, AuthError>;
    /// Inserts a user. Returns [`AuthError::Conflict`] when the username or the
    /// external identity is already taken.
    async fn create_user(&self, user: User) -> Result<(), AuthError>;
    async fn find_user_by_external(
        &self,
        source: &str,
        external_id: &str,
    ) -> Result<Option<User>, AuthError>;
    async fn update_user_profile(
        &self,
        id: &str,
        email: Option<&str>,
        display_name: Option<&str>,
        is_admin: bool,
    ) -> Result<(), AuthError>;
    /// Replaces the user's memberships in the external namespace with
    /// `groups`. Local memberships are untouched.
    async fn set_external_groups(&self, user_id: &str, groups: &[String]) -> Result<(), AuthError>;
    /// All groups of the user across both namespaces.
    async fn user_groups(&self, user_id: &str) -> Result<Vec<Group>, AuthError>;
    async fn find_user_by_username(&self, username: &str) -> Result<Option<User>, AuthError>;
    async fn find_user_by_id(&self, id: &str) -> Result<Option<User>, AuthError>;

    async fn create_session(&self, session: Session) -> Result<(), AuthError>;
    async fn find_session(&self, id_hash: &str) -> Result<Option<Session>, AuthError>;
    async fn delete_session(&self, id_hash: &str) -> Result<(), AuthError>;

    async fn insert_client(&self, client: OAuthClient) -> Result<(), AuthError>;
    async fn find_client(&self, client_id: &str) -> Result<Option<OAuthClient>, AuthError>;

    async fn insert_code(&self, code: AuthCode) -> Result<(), AuthError>;
    /// Atomically removes and returns the code, making it single-use.
    async fn take_code(&self, code_hash: &str) -> Result<Option<AuthCode>, AuthError>;

    async fn insert_access_token(&self, token: AccessToken) -> Result<(), AuthError>;
    async fn find_access_token(&self, token_hash: &str) -> Result<Option<AccessToken>, AuthError>;
    async fn insert_refresh_token(&self, token: RefreshToken) -> Result<(), AuthError>;
    /// Atomically removes and returns the refresh token (rotation).
    async fn take_refresh_token(&self, token_hash: &str)
        -> Result<Option<RefreshToken>, AuthError>;
    /// Deletes an access or refresh token if it exists.
    async fn revoke_token(&self, token_hash: &str) -> Result<(), AuthError>;

    async fn insert_oidc_login(&self, login: OidcLogin) -> Result<(), AuthError>;
    /// Atomically removes and returns the login, making the callback single-use.
    async fn take_oidc_login(&self, state_hash: &str) -> Result<Option<OidcLogin>, AuthError>;

    /// Removes expired codes, tokens and sessions.
    async fn purge_expired(&self, now: i64) -> Result<(), AuthError>;
}
