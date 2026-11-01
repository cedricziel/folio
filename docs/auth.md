# Authentication

Folio ships its own OAuth 2.1 authorization server. It protects the MCP
endpoint today and is meant to serve the REST API and the native apps later.
The web UI and `/api` require a login (session cookie) or an API-scoped bearer
token. Set `auth.required = false` (`FOLIO_AUTH__REQUIRED=false`) to turn that
off. Exempt either way: `/healthz`, `/readyz`, `/static`, `/login`, `/oauth/*`,
`/.well-known/*`, `/manifest.webmanifest`, `/sw.js`, `/api/openapi.json`,
`/api/docs`, `/setup` (while no user exists, with the setup token) and `/mcp`
(guarded by its own bearer layer).

Bearer tokens for `/api` must carry `documents:read` for `GET`, `HEAD` and
`OPTIONS` and `documents:write` for everything else; a token without the needed
scope gets `403` with `error="insufficient_scope"`. Cookie-authenticated writes
must come from the same origin (`Sec-Fetch-Site`, or `Origin` on older browsers).

## First run

While no user exists, anyone who can reach the server could claim the owner
account, so `/setup` needs a one-time token. Folio prints the full link in its
log at startup:

```
No owner account exists yet. Open this link to create it:

  https://folio.example.com/setup?token=...
```

Open it and create the owner account (username and a password of 12 to 1024
characters). Without the token `/setup` only says to look in the server log.
Set `FOLIO_AUTH__SETUP_TOKEN` (or `FOLIO_AUTH__SETUP_TOKEN_FILE`) to choose the
token yourself, for example in provisioning scripts; otherwise a new random one
is generated on every start. The token stops mattering once a user exists, and
`/setup` then redirects to `/login`. Passwords are stored as argon2id hashes.

## Public URL

Set the externally visible base URL:

```toml
[server]
public_url = "https://folio.example.com"
```

or `FOLIO_SERVER__PUBLIC_URL=https://folio.example.com`. The issuer, OAuth and
resource metadata, token audiences, `WWW-Authenticate` challenges and the
cookie `Secure` flag are derived from it and nothing else: `Host` and
`X-Forwarded-*` are never read. With auth enabled Folio therefore refuses to
start when it listens on a non-loopback address (for example `0.0.0.0`)
without `public_url`. On a loopback address it falls back to
`http://<host>:<port>`.

Over https the session and CSRF cookies use the `__Host-` prefix and the
`Secure` flag. Forms carry an HMAC CSRF token (bound to the session once logged
in) and the auth pages send `X-Frame-Options: DENY`, `frame-ancestors 'none'`
and `Cache-Control: no-store`. Authorization responses include `iss`
(RFC 9207).

## Limits

- Login: after 5 failures for a username (10 for a client IP) further attempts
  get `429` with `Retry-After`, starting at 30 seconds and doubling up to 15
  minutes. A username lockout also blocks the real owner for that time. At most
  4 `argon2` hashes run at once and passwords over 1024 characters are
  rejected.
- `POST /oauth/register`: 16 KiB body, at most 10 `redirect_uris` of 2 KiB each,
  `client_name` up to 200 characters, and 20 registrations per peer address
  (refilled over an hour). The peer is the TCP address, so behind a reverse
  proxy all clients share the proxy's limit. Clients that never obtained a token
  are purged after 24 hours; expired codes, tokens and sessions are purged every
  ten minutes.
- Tokens issued from one authorization belong to a grant. Presenting an
  already-rotated refresh token, or revoking any token of a grant, ends the
  whole grant. `POST /oauth/revoke` needs `token` and `client_id`; tokens of
  other clients are ignored.
- Installed-app sharing (`POST /share`) while signed out shows a page asking the
  user to sign in and share again instead of buffering the upload.

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
| `/.well-known/oauth-protected-resource[/mcp\|/api]` | RFC 9728 metadata; `/api` challenges point at the `/api` document |
| `POST /oauth/register` | RFC 7591 dynamic client registration (public clients) |
| `/oauth/authorize` | authorization code with PKCE (S256 only), consent page |
| `POST /oauth/token` | `authorization_code` and `refresh_token` grants |
| `POST /oauth/revoke` | RFC 7009 revocation (`token`, `client_id`) |

Scopes are `documents:read` and `documents:write` (both by default). Tokens
are opaque, stored hashed, and bound to the `resource` (RFC 8707), which must
be `<public_url>/mcp` or `<public_url>/api`.

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
