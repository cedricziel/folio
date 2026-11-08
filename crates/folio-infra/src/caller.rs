//! [`CallerResolver`] backed by folio-auth accounts

use std::sync::Arc;

use async_trait::async_trait;
use folio_auth::AuthStore;
use folio_core::error::{Error, Result};
use folio_core::models::{Caller, UserId};
use folio_core::ports::CallerResolver;

/// Reads the admin and owner flags of an account
///
/// With authentication disabled every caller is an admin, the same rule the
/// settings pages follow.
pub struct AuthCallerResolver {
    store: Arc<dyn AuthStore>,
    required: bool,
}

impl AuthCallerResolver {
    pub fn new(store: Arc<dyn AuthStore>, required: bool) -> Self {
        Self { store, required }
    }
}

#[async_trait]
impl CallerResolver for AuthCallerResolver {
    async fn resolve(&self, user: Option<&UserId>) -> Result<Caller> {
        if !self.required {
            return Ok(Caller {
                user_id: user.cloned(),
                is_admin: true,
                is_owner: false,
            });
        }
        let Some(user) = user else {
            return Ok(Caller::unprivileged(None));
        };
        let account = self
            .store
            .find_user_by_id(&user.0)
            .await
            .map_err(|e| Error::Repository(e.to_string()))?;
        Ok(match account {
            Some(account) => Caller {
                user_id: Some(user.clone()),
                is_admin: account.is_admin,
                is_owner: account.is_owner,
            },
            None => Caller::unprivileged(Some(user.clone())),
        })
    }
}
