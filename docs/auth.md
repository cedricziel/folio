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

## Login methods

| Method | Enabled by | Notes |
| --- | --- | --- |
| Username and password | always | The owner account from `/setup`. |
| Forward auth (trusted headers) | `auth.forward.enabled` | For deployments behind an SSO gateway. See below. |

All methods end in the same session cookie, so the OAuth authorize flow, the
web UI and the MCP login work the same way with any of them.

## Forward auth

A reverse proxy or SSO gateway authenticates the user and passes the identity
in headers. Folio logs that user in without asking for a password.

> **Every header Folio reads must be overwritten or stripped by your proxy.**
> Folio cannot tell a header set by the proxy from one a client sent along: if
> the proxy forwards a client-supplied copy of any header you configured, that
> client can impersonate any user, or become an admin. Configure only headers
> your proxy sets itself on every authenticated request, and check that it
> removes incoming copies of them.

```toml
[auth.forward]
enabled = true
trusted_proxies = ["172.18.0.5"]   # a single proxy address, see Trust below
# user_header = "Remote-User"      # defaults: Remote-User, Remote-Email,
# email_header = "Remote-Email"    # Remote-Name
# name_header = "Remote-Name"
# groups_header = "Remote-Groups"  # off unless set
# admin_group = "folio-admins"     # needs groups_header
# role_header = "Remote-Role"      # off unless set
# admin_role = "admin"             # needs role_header
```

Environment: `FOLIO_AUTH__FORWARD__ENABLED=true`,
`FOLIO_AUTH__FORWARD__TRUSTED_PROXIES=10.0.0.0/8,172.16.0.5`, and so on.

- **Trust.** Headers are honoured only when the TCP peer address is in
  `trusted_proxies`. From any other peer they are removed and a warning is
  logged. Do not list a range that untrusted clients can reach, and make sure
  your proxy overwrites these headers instead of passing client values through.
  With the list empty nothing is trusted.
- **Users.** The first request for a name creates the account (no local
  password) and links it by name. A name that already belongs to another
  account, such as the local owner, is refused with `403`.
- **Admins.** The first user of a fresh install is the owner and admin. With
  `admin_group` (and `groups_header`) or `admin_role` (and `role_header`) set,
  admin status follows that mapping on every request. Without a mapping only
  the first user is admin. A mapping whose header is not configured is ignored.
- **Groups.** Off by default. With `groups_header` set, that header replaces
  the user's external groups on every request. External groups and local
  groups are separate namespaces: an IdP group `admins` never matches a local
  group `admins`.
- **Sessions.** A trusted request sets a normal session cookie. After that the
  cookie works on its own, so API clients and MCP clients still use OAuth
  tokens, which are not affected by the headers.

### Pangolin

When SSO is enabled on a resource, Pangolin (Badger) sends `Remote-User`,
`Remote-Email`, `Remote-Name`, `Remote-Role` and `Remote-User-Id`. It does
**not** send groups, and it does **not** strip a `Remote-Groups` header a
client sends. So do not set `groups_header` or `admin_group` for Pangolin:
any signed-in user could send `Remote-Groups: folio-admins` and become an
admin. Map admins by role instead:

```yaml
services:
  folio:
    environment:
      FOLIO_SERVER__PUBLIC_URL: https://folio.example.com
      FOLIO_AUTH__FORWARD__ENABLED: "true"
      FOLIO_AUTH__FORWARD__TRUSTED_PROXIES: 172.18.0.5   # the Newt container
      FOLIO_AUTH__FORWARD__ROLE_HEADER: Remote-Role
      FOLIO_AUTH__FORWARD__ADMIN_ROLE: admin
```

Pangolin reaches Folio from its Newt/Gerbil network, so trust that address.
Enable SSO on the Pangolin resource. Add the `/mcp` and `/oauth/*` paths as
auth-bypass rules, because MCP clients cannot sign in through the gateway and
use Folio's own OAuth instead.

### Other gateways

- **Authelia** (Caddy, nginx, Traefik forward auth): the same user, name and
  email defaults apply; set `groups_header = "Remote-Groups"` to use groups.
- **Authentik proxy outpost**: set `user_header = "X-authentik-username"`,
  `email_header = "X-authentik-email"`, `name_header = "X-authentik-name"`,
  `groups_header = "X-authentik-groups"` (groups are `|` separated).
- **oauth2-proxy**: pass `--set-xauthrequest` and map `X-Auth-Request-User`,
  `X-Auth-Request-Email`, `X-Auth-Request-Groups`.

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
