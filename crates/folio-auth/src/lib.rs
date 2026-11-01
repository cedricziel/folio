//! Folio authentication: users, sessions and an OAuth 2.1 authorization server.
//!
//! This crate holds the domain types, the [`AuthStore`] port and the protocol
//! logic. Persistence lives in `folio-infra`, HTTP routes in `folio-api`.
//!
//! Resource servers (the MCP endpoint, later the REST API) only need
//! [`TokenValidator`] and [`require_bearer`].

pub mod crypto;
pub mod external;
pub mod forward;
pub mod model;
pub mod redirect;
pub mod server;
pub mod store;
pub mod validator;

pub use forward::{ForwardAuth, ForwardConfig};
pub use model::{
    AccessToken, AuthCode, AuthContext, Group, GroupSource, OAuthClient, RefreshToken, Session,
    User, SCOPES,
};
pub use store::{AuthError, AuthStore};
pub use validator::{base_url, require_bearer, TokenValidator};
