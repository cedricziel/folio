//! Persistence port for the auth domain.

use async_trait::async_trait;

use crate::model::{
    AccessToken, AuthCode, ConnectedClient, Group, OAuthClient, OidcLogin, PasskeyRecord,
    PersonalToken, RefreshToken, Session, TotpRecord, User,
};

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("storage error: {0}")]
    Store(String),
    #[error("already exists")]
    Conflict,
}

/// The tokens a grant exchange stores.
#[derive(Debug, Clone)]
pub struct IssuedTokens {
    pub access: AccessToken,
    pub refresh: RefreshToken,
}

/// Runs inside an exchange transaction on the grant it took. Returning `None`
/// rejects the grant: it stays consumed and no tokens are stored.
pub type GrantCheck<'a, G> = Box<dyn FnOnce(&G) -> Option<IssuedTokens> + Send + 'a>;

/// What happened to a refresh token presented for rotation.
#[derive(Debug, Clone)]
pub enum RefreshExchange {
    /// The token was unused and is now rotated.
    Rotated(RefreshToken),
    /// The token was already rotated. It leaked, so its whole grant was revoked.
    Replayed,
    /// No such token.
    Unknown,
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
    /// Like [`Self::create_user`]; the user becomes owner and admin when no
    /// user exists yet. The check is atomic with the insert, so parallel first
    /// requests cannot all become owner.
    async fn create_external_user(&self, user: User) -> Result<(), AuthError>;
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
    /// Remembers the OpenID Connect ID token that created the session.
    async fn set_session_id_token(&self, id_hash: &str, id_token: &str) -> Result<(), AuthError>;
    async fn session_id_token(&self, id_hash: &str) -> Result<Option<String>, AuthError>;

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
    /// Takes the code like [`Self::take_code`] and stores the tokens `check`
    /// returns, marking `client_id` used, in one transaction serialized with
    /// [`Self::revoke_client_tokens`] for that client. Tokens issued here
    /// therefore never outlive a disconnect. Returns the taken code.
    ///
    /// The tokens must belong to `client_id`. If storing them or committing
    /// fails, the whole transaction rolls back and the code stays usable.
    async fn exchange_code(
        &self,
        client_id: &str,
        code_hash: &str,
        check: GrantCheck<'_, AuthCode>,
    ) -> Result<Option<AuthCode>, AuthError>;

    async fn insert_personal_token(&self, token: PersonalToken) -> Result<(), AuthError>;
    async fn find_personal_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalToken>, AuthError>;
    /// All credentials owned by the user, newest first, including revoked ones.
    async fn list_personal_tokens(&self, user_id: &str) -> Result<Vec<PersonalToken>, AuthError>;
    /// Revokes only the owner's credential. Returns false when it does not exist.
    async fn revoke_personal_token(
        &self,
        user_id: &str,
        id: &str,
        at: i64,
    ) -> Result<bool, AuthError>;

    async fn insert_access_token(&self, token: AccessToken) -> Result<(), AuthError>;
    async fn find_access_token(&self, token_hash: &str) -> Result<Option<AccessToken>, AuthError>;
    async fn insert_refresh_token(&self, token: RefreshToken) -> Result<(), AuthError>;
    /// Atomically marks the refresh token as used and returns it (rotation).
    /// A token that was already used returns `None`.
    async fn take_refresh_token(&self, token_hash: &str)
        -> Result<Option<RefreshToken>, AuthError>;
    /// Rotates the refresh token like [`Self::take_refresh_token`] and stores
    /// the tokens `check` returns, with the guarantees of [`Self::exchange_code`].
    /// A token that was already rotated revokes its grant under the lock of the
    /// client that owns it, so a rotation racing the replay cannot survive it.
    async fn exchange_refresh_token(
        &self,
        client_id: &str,
        token_hash: &str,
        check: GrantCheck<'_, RefreshToken>,
    ) -> Result<RefreshExchange, AuthError>;
    /// The grant of a refresh token that was already rotated, to detect reuse.
    async fn used_refresh_grant(&self, token_hash: &str) -> Result<Option<String>, AuthError>;
    /// Deletes every access and refresh token of a grant.
    async fn revoke_grant(&self, grant_id: &str) -> Result<(), AuthError>;
    /// Revokes the grant of an access or refresh token issued to `client_id`.
    /// Unknown tokens and tokens of other clients are ignored.
    async fn revoke_token(&self, token_hash: &str, client_id: &str) -> Result<(), AuthError>;

    async fn insert_oidc_login(&self, login: OidcLogin) -> Result<(), AuthError>;
    /// Atomically removes and returns the login, making the callback single-use.
    async fn take_oidc_login(&self, state_hash: &str) -> Result<Option<OidcLogin>, AuthError>;

    /// Live sessions of the user, newest first.
    async fn list_sessions(&self, user_id: &str, now: i64) -> Result<Vec<Session>, AuthError>;
    /// Deletes the user's sessions, except `keep` when given.
    async fn delete_user_sessions(
        &self,
        user_id: &str,
        keep: Option<&str>,
    ) -> Result<(), AuthError>;
    async fn set_password_hash(&self, user_id: &str, hash: &str) -> Result<(), AuthError>;

    /// Clients that hold unexpired tokens of the user.
    async fn list_connected_clients(
        &self,
        user_id: &str,
        now: i64,
    ) -> Result<Vec<ConnectedClient>, AuthError>;
    /// Deletes every token and pending authorization code the client holds for
    /// the user. Serialized with the exchanges of the same client, so a token
    /// exchange either finishes before it (and loses its tokens) or starts after
    /// it (and finds its grant gone). Consent is not serialized: a user who
    /// approves the client while disconnecting it may end up with a fresh code.
    async fn revoke_client_tokens(&self, user_id: &str, client_id: &str) -> Result<(), AuthError>;

    async fn get_totp(&self, user_id: &str) -> Result<Option<TotpRecord>, AuthError>;
    /// Stores an unconfirmed seed, replacing any earlier unconfirmed one. A
    /// confirmed enrollment is left alone; returns whether the seed was stored.
    async fn save_pending_totp(
        &self,
        user_id: &str,
        secret_enc: &str,
        now: i64,
    ) -> Result<bool, AuthError>;
    /// Marks the pending enrollment confirmed at `step` and replaces the
    /// recovery codes with `recovery_hashes`, in one transaction. Returns
    /// false when there is no pending enrollment.
    async fn confirm_totp(
        &self,
        user_id: &str,
        expected_secret: &str,
        step: i64,
        recovery_hashes: &[String],
    ) -> Result<bool, AuthError>;
    /// Moves the used step forward. Returns false when `step` is not newer
    /// than the stored one, which makes the check-and-set atomic.
    async fn advance_totp_step(&self, user_id: &str, step: i64) -> Result<bool, AuthError>;
    /// Removes the enrollment and every recovery code.
    async fn delete_totp(&self, user_id: &str) -> Result<(), AuthError>;
    /// Cancel only an unconfirmed enrollment; a confirmed factor is preserved.
    async fn delete_pending_totp(&self, user_id: &str) -> Result<bool, AuthError>;
    async fn replace_recovery_codes(
        &self,
        user_id: &str,
        hashes: &[String],
    ) -> Result<(), AuthError>;
    /// Deletes the code; true when it existed, so a code works once.
    async fn use_recovery_code(&self, user_id: &str, hash: &str) -> Result<bool, AuthError>;
    async fn count_recovery_codes(&self, user_id: &str) -> Result<i64, AuthError>;

    async fn list_passkeys(&self, user_id: &str) -> Result<Vec<PasskeyRecord>, AuthError>;
    /// [`AuthError::Conflict`] when the credential id is already registered.
    async fn insert_passkey(&self, passkey: PasskeyRecord) -> Result<(), AuthError>;
    /// Stores the updated credential (signature counter) after a login.
    async fn touch_passkey(&self, id: &str, data: &str, used_at: i64) -> Result<(), AuthError>;
    async fn rename_passkey(&self, user_id: &str, id: &str, name: &str) -> Result<bool, AuthError>;
    async fn delete_passkey(&self, user_id: &str, id: &str) -> Result<bool, AuthError>;

    /// Removes expired codes, tokens and sessions.
    async fn purge_expired(&self, now: i64) -> Result<(), AuthError>;
}
