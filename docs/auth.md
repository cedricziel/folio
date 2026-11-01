# Authentication

Folio ships its own OAuth 2.1 authorization server. It protects the MCP
endpoint today and is meant to serve the REST API and the native apps later.
The web UI and `/api` require a login (session cookie) or an API-scoped bearer
token. Set `auth.required = false` (`FOLIO_AUTH__REQUIRED=false`) to turn that
off. Exempt either way: `/healthz`, `/readyz`, `/static`, `/login`, `/oauth/*`,
`/.well-known/*`, `/api/openapi.json`, `/api/docs`, `/setup` (while no user
exists) and `/mcp` (guarded by its own bearer layer).

## First run

Open `/setup` while no user exists and create the owner account (username and
a password of at least 12 characters). `/setup` redirects to `/login` once a
user exists. Passwords are stored as argon2id hashes.

## Public URL

Set the externally visible base URL so metadata and redirects are correct
behind a reverse proxy:

```toml
[server]
public_url = "https://folio.example.com"
```

or `FOLIO_SERVER__PUBLIC_URL=https://folio.example.com`. Without it Folio uses
`X-Forwarded-Proto` and the `Host` header, which is fine for `localhost`.
Session cookies get the `Secure` flag when the base URL is https.

## Connecting an MCP client

Add `https://folio.example.com/mcp` as a remote MCP server in Claude Desktop,
Claude Code or claude.ai. The client discovers the authorization server from
`/.well-known/oauth-protected-resource`, registers itself at
`/oauth/register`, and opens `/oauth/authorize`. Sign in, review the consent
page and allow. Access tokens last one hour; clients renew them with a
rotating refresh token.

## Endpoints

| Path | Purpose |
| --- | --- |
| `/.well-known/oauth-authorization-server` | RFC 8414 metadata |
| `/.well-known/oauth-protected-resource[/mcp]` | RFC 9728 metadata |
| `POST /oauth/register` | RFC 7591 dynamic client registration (public clients) |
| `/oauth/authorize` | authorization code with PKCE (S256 only), consent page |
| `POST /oauth/token` | `authorization_code` and `refresh_token` grants |
| `POST /oauth/revoke` | RFC 7009 revocation |

Scopes are `documents:read` and `documents:write` (both by default). Tokens
are opaque, stored hashed, and bound to the `resource` (RFC 8707), which must
be `<public_url>/mcp`.

## Protecting a route

```rust
let validator = auth_state.validator("/mcp");
let mcp = mcp_router.layer(axum::middleware::from_fn_with_state(
    validator,
    folio_auth::require_bearer,
));
```

Missing or invalid tokens get `401` with
`WWW-Authenticate: Bearer resource_metadata="<public_url>/.well-known/oauth-protected-resource"`.
Handlers can read the caller from the `folio_auth::AuthContext` request
extension. For cookie-based pages use the `folio_api::RequireUser` extractor.
