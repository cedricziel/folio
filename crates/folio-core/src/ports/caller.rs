//! Resolving the caller's instance-wide permissions

use async_trait::async_trait;

use crate::error::Result;
use crate::models::{Caller, UserId};

/// Looks up whether an account is an instance admin, for the API and MCP alike
#[async_trait]
pub trait CallerResolver: Send + Sync {
    /// `user` is the signed-in account, or `None` for an anonymous request
    async fn resolve(&self, user: Option<&UserId>) -> Result<Caller>;
}
