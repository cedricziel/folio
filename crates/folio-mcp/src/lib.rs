//! Folio MCP server
//!
//! Exposes the archive to AI assistants through the Model Context Protocol,
//! over streamable HTTP (mounted into the axum app) or stdio.

mod auth;
mod server;
mod skills;
mod transport;

pub use auth::{bridge_auth_context, McpAuth, SCOPE_READ, SCOPE_WRITE};
pub use server::FolioMcp;
pub use transport::{is_enabled, router, serve_stdio, McpOptions, ENABLED_SETTING};
