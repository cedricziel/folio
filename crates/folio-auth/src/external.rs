//! Linking and provisioning of users who authenticate elsewhere (forward
//! auth, later OIDC).

use crate::{
    model::User,
    server::now,
    store::{AuthError, AuthStore},
};

const MAX_USERNAME_LEN: usize = 64;
const MAX_GROUPS: usize = 200;
const MAX_GROUP_LEN: usize = 200;

#[derive(Debug, Clone)]
pub struct ExternalIdentity {
    /// Provider kind, for example [`crate::model::SOURCE_FORWARD`].
    pub source: String,
    /// Stable identifier at the provider.
    pub external_id: String,
    pub username: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub groups: Vec<String>,
    /// `Some` when the provider decides admin status (group or role mapping);
    /// `None` leaves it to the first-user rule and manual changes.
    pub admin: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExternalError {
    #[error("the username belongs to another account")]
    UsernameTaken,
    #[error("the identity is not usable")]
    Invalid,
    #[error(transparent)]
    Store(#[from] AuthError),
}

/// Finds or creates the local user for an external identity, refreshes its
/// profile and replaces its external group memberships.
///
/// With `identity.admin` set, admin status follows it on every call. Without
/// it, only the very first user becomes admin.
pub async fn sync_external_user(
    store: &dyn AuthStore,
    mut identity: ExternalIdentity,
) -> Result<User, ExternalError> {
    identity.username = identity.username.trim().to_string();
    let name = &identity.username;
    if name.is_empty() || name.len() > MAX_USERNAME_LEN || name.chars().any(char::is_control) {
        return Err(ExternalError::Invalid);
    }
    identity
        .groups
        .retain(|g| !g.is_empty() && g.len() <= MAX_GROUP_LEN);
    identity.groups.truncate(MAX_GROUPS);

    let user = match store
        .find_user_by_external(&identity.source, &identity.external_id)
        .await?
    {
        Some(mut user) => {
            let is_admin = identity.admin.unwrap_or(user.is_admin);
            if user.email != identity.email
                || user.display_name != identity.display_name
                || user.is_admin != is_admin
            {
                store
                    .update_user_profile(
                        &user.id,
                        identity.email.as_deref(),
                        identity.display_name.as_deref(),
                        is_admin,
                    )
                    .await?;
                user.email = identity.email.clone();
                user.display_name = identity.display_name.clone();
                user.is_admin = is_admin;
            }
            user
        }
        None => {
            let first = store.count_users().await? == 0;
            let user = User {
                id: uuid::Uuid::new_v4().to_string(),
                username: identity.username.clone(),
                password_hash: crate::model::NO_PASSWORD.to_string(),
                created_at: now(),
                email: identity.email.clone(),
                display_name: identity.display_name.clone(),
                is_admin: identity.admin.unwrap_or(false) || first,
                auth_source: identity.source.clone(),
                external_id: Some(identity.external_id.clone()),
            };
            match store.create_user(user.clone()).await {
                Ok(()) => user,
                // Either a concurrent first request created this very identity
                // (use it) or the username belongs to someone else.
                Err(AuthError::Conflict) => store
                    .find_user_by_external(&identity.source, &identity.external_id)
                    .await?
                    .ok_or(ExternalError::UsernameTaken)?,
                Err(e) => return Err(e.into()),
            }
        }
    };
    store
        .set_external_groups(&user.id, &identity.groups)
        .await?;
    Ok(user)
}
