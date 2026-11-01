//! Authorization context supplied by whichever auth layer fronts `/mcp`

use std::collections::HashSet;

use rmcp::model::Extensions;
use rmcp::service::RequestContext;
use rmcp::RoleServer;

/// Scope required by tools that only read the archive
pub const SCOPE_READ: &str = "documents:read";
/// Scope required by tools that change the archive
pub const SCOPE_WRITE: &str = "documents:write";

/// Identity and scopes of the caller
///
/// An auth layer in front of the MCP router inserts this into the HTTP
/// request extensions after validating the bearer token. When no value is
/// present (no auth layer, or the stdio transport) the caller is trusted.
#[derive(Debug, Clone)]
pub struct McpAuth {
    pub subject: Option<String>,
    pub scopes: HashSet<String>,
}

impl McpAuth {
    pub fn new(subject: Option<String>, scopes: HashSet<String>) -> Self {
        Self { subject, scopes }
    }
}

/// Middleware that turns the caller validated by `folio_auth::require_bearer`
/// into the [`McpAuth`] the tools check. Layer it inside `require_bearer`.
pub async fn bridge_auth_context(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if let Some(context) = request.extensions().get::<folio_auth::AuthContext>() {
        let auth = McpAuth::new(
            Some(context.user_id.clone()),
            context.scopes.iter().cloned().collect(),
        );
        request.extensions_mut().insert(auth);
    }
    next.run(request).await
}

/// Fail unless the request is allowed to use `scope`
pub(crate) fn require_scope(ctx: &RequestContext<RoleServer>, scope: &str) -> Result<(), String> {
    match auth_of(&ctx.extensions) {
        Some(auth) if !auth.scopes.contains(scope) => {
            Err(format!("insufficient scope: this tool requires '{scope}'"))
        }
        _ => Ok(()),
    }
}

fn auth_of(extensions: &Extensions) -> Option<&McpAuth> {
    extensions
        .get::<http::request::Parts>()?
        .extensions
        .get::<McpAuth>()
}
