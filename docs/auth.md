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

## Login methods

| Method | Enabled by | Notes |
| --- | --- | --- |
| Username and password | always | The owner account from `/setup`. |
| Forward auth (trusted headers) | `auth.forward.enabled` | For deployments behind an SSO gateway. See below. |
| OpenID Connect | `auth.oidc.enabled` | A "Sign in with SSO" button. See [OpenID Connect](#openid-connect). |

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
# user_id_header = "Remote-User-Id" # stable id, recommended when available
# user_header = "Remote-User"      # defaults: Remote-User, Remote-Email,
# email_header = "Remote-Email"    # Remote-Name
# name_header = "Remote-Name"
# groups_header = "Remote-Groups"  # off unless set
# group_separator = ","            # between group names
# admin_group = "folio-admins"     # needs groups_header
# role_header = "Remote-Role"      # off unless set
# admin_role = "admin"             # needs role_header
```

Environment: `FOLIO_AUTH__FORWARD__ENABLED=true`,
`FOLIO_AUTH__FORWARD__TRUSTED_PROXIES=10.0.0.0/8,172.16.0.5`, and so on.

- **Trust.** Headers are honoured only when the TCP peer address is in
  `trusted_proxies`. From any other peer they are removed and a warning is
  logged. Trust a single proxy address (the one Folio sees as the peer), never
  a broad range, and do not publish Folio's port: clients must only be able to
  reach it through the proxy. Folio logs a warning when the list contains
  `0.0.0.0/0` or `::/0`, which would let every client impersonate any user.
  With the list empty nothing is trusted.
- **Shared secret.** Optionally set `shared_secret` (and `secret_header`,
  default `X-Folio-Proxy-Secret`) and have the proxy add that header to every
  request. Folio then treats requests without the right value as
  unauthenticated, which protects you if the proxy's address is ever reachable
  by someone else. The secret header is removed before the request is handled.
- **Machine paths.** Identity headers are never honoured on `/oauth/token`,
  `/oauth/register`, `/oauth/revoke`, `/.well-known/*`, `/mcp` and on `/api`
  requests that carry a bearer token. Those use OAuth, not the gateway.
- **Users.** The first request for a name creates the account (no local
  password) and links it by name. With `user_id_header` set, accounts are
  linked by that stable id instead, and the user header is only the username
  shown in Folio (it is taken from the first request and does not follow later
  renames). Prefer this whenever your gateway offers an id: names can be
  reused, ids cannot. A name that already belongs to another
  account, such as the local owner, is refused with `403`.
- **Admins.** The first user of a fresh install is the owner and admin. With
  `admin_group` (and `groups_header`) or `admin_role` (and `role_header`) set,
  admin status follows that mapping on every request. Without a mapping only
  the first user is admin. A mapping whose header is not configured is ignored.
- **Groups.** Off by default. With `groups_header` set, that header replaces
  the user's external groups on every request. External groups and local
  groups are separate namespaces: an IdP group `admins` never matches a local
  group `admins`.
- **Sessions.** A trusted page load (a request that accepts `text/html`) sets
  a normal session cookie that lasts `session_ttl_secs` (12 hours by default,
  shorter than the 14 days of a local login) and is reused on later requests.
  Other cookieless requests, such as scripts that only send the headers, get
  no session and are not logged in; API and MCP clients use OAuth tokens,
  which the headers never affect.
- **Logout.** Folio's own sign-out only deletes the session; the next request
  through the proxy signs the user in again while the gateway session lives.
  Sign out at the gateway. Once the gateway session ends, the proxy stops
  sending the identity and Folio deletes the forward-auth session on the next
  trusted request that arrives without it. Accounts that came from the gateway
  have no local password, so they can only sign in through it.

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
      FOLIO_AUTH__FORWARD__USER_ID_HEADER: Remote-User-Id
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
  `groups_header = "X-authentik-groups"`, `group_separator = "|"`.
- **oauth2-proxy**: pass `--set-xauthrequest` and map `X-Auth-Request-User`,
  `X-Auth-Request-Email`, `X-Auth-Request-Groups`.

## OpenID Connect

Folio can act as an OpenID Connect relying party, so one identity provider
(Authentik, Authelia, Pocket ID, Keycloak, ...) signs everybody in. The login
page gets a button; the rest (sessions, the MCP and native app OAuth flow,
`next` redirects) works as with a password.

```toml
[auth.oidc]
enabled = true
issuer = "https://auth.example.com/application/o/folio/"
client_id = "folio"
client_secret = "..."          # or FOLIO_AUTH__OIDC__CLIENT_SECRET_FILE; omit for a public client
admin_group = "folio-admins"   # optional
# scopes = "openid profile email"
# display_name = "Sign in with SSO"
# groups_claim = "groups"
# allow_local_login = true
# auto_create_users = true
```

Register `<public_url>/auth/oidc/callback` as the redirect URI at the provider
and set `server.public_url`; the redirect URI Folio sends must match exactly.
The issuer must be `https` (plain `http` only for loopback) and must equal the
`issuer` of `<issuer>/.well-known/openid-configuration`, including a trailing
slash if the provider has one.

- **Flow.** Authorization code with PKCE (S256), `state` and `nonce`. The
  pending login is stored server-side for ten minutes, bound to the browser by
  a cookie, and can be used once.
- **Validation.** The ID token must be signed by a key from the provider's
  JWKS and carry the right issuer, audience, nonce and a valid
  `exp` (one minute of clock leeway). An unknown `kid` makes Folio refetch the
  JWKS at most every ten seconds, so key rotation needs no restart. Discovery
  is cached for an hour. If the ID token lacks `preferred_username` or the
  groups claim and the provider has a userinfo endpoint, Folio asks it.
- **Algorithms.** Only asymmetric signatures are accepted: RS256/384/512,
  PS256/384/512, ES256/384/512 and EdDSA. HMAC (`HS*`) ID tokens are refused,
  because anyone who knows the client secret could forge them. Set the
  provider's signing key to RSA or ECDSA (in Authentik, pick a signing key,
  not "HS256").
- **Endpoints.** The authorization, token, userinfo and JWKS URLs from
  discovery must be https (loopback excepted).
- **Rate limit.** One client address can start 30 sign-ins per minute; behind a
  reverse proxy that address is the proxy's.
- **Users.** An account is linked by issuer and `sub`, never by name or email.
  The username comes from `preferred_username`, then `email`, then `sub`. A
  name that already belongs to a local or forward-auth account is refused with
  `403`. With `auto_create_users = false` only users who already signed in
  once can sign in, except that the very first sign-in on an empty install
  still creates the owner.
- **Admins and groups.** The first user of a fresh install is the owner. With
  `admin_group` set, admin status follows membership of that group on every
  login. The groups claim replaces the user's external groups on every login.
- **Local login.** With `allow_local_login = false` the password form is gone
  and refused, and `/setup` (and its token) is not used: the first person to sign in
  through the provider becomes owner and admin. With local login on, an empty
  install still starts at `/setup`, which also offers the SSO button. Keep it `true` until SSO works, so you
  cannot lock yourself out.
- **Logout.** If the provider advertises an `end_session_endpoint`, signing out
  of Folio also ends the session at the provider (RP-initiated logout). Folio
  keeps the ID token with the session and sends it as `id_token_hint`, and asks
  the provider to return to `<public_url>/login`; register that URL as an
  allowed post-logout redirect.

### Authentik

1. Create a provider of type **OAuth2/OpenID Provider**. Client type
   *Confidential*, redirect URI (strict) `https://folio.example.com/auth/oidc/callback`,
   signing key any RSA key. Note the client ID and secret.
2. Create an application with slug `folio` and attach the provider. The issuer
   is `https://auth.example.com/application/o/folio/`.
3. Add `https://folio.example.com/login` to the provider's allowed redirect
   URIs too: Authentik uses that list for the post-logout redirect.
4. Groups: the default `profile` scope mapping already adds a `groups` claim
   with the user's group names. Create a group `folio-admins` and add admins to
   it, then set `admin_group = "folio-admins"`.

### Authelia

Add a client under `identity_providers.oidc.clients` with
`client_id: folio`, a hashed `client_secret`,
`redirect_uris: [https://folio.example.com/auth/oidc/callback]`,
`scopes: [openid, profile, email, groups]`, `authorization_policy` as you like.
The issuer is your Authelia URL, for example `https://auth.example.com`. Set
`scopes = "openid profile email groups"` in Folio so the `groups` claim is
released.

### Pocket ID

Create an OIDC client with callback URL
`https://folio.example.com/auth/oidc/callback`, copy the client ID and secret,
and use your Pocket ID URL as the issuer. Pocket ID puts a user's groups in the
`groups` claim; add the groups to the client's allowed groups and set
`admin_group` to the admin group's name.

## Endpoints

| Path | Purpose |
| --- | --- |
| `/.well-known/oauth-authorization-server` | RFC 8414 metadata |
| `/.well-known/oauth-protected-resource[/mcp\|/api]` | RFC 9728 metadata; `/api` challenges point at the `/api` document |
| `POST /oauth/register` | RFC 7591 dynamic client registration (public clients) |
| `/oauth/authorize` | authorization code with PKCE (S256 only), consent page |
| `POST /oauth/token` | `authorization_code` and `refresh_token` grants |
| `POST /oauth/revoke` | RFC 7009 revocation (`token`, `client_id`) |
| `GET /auth/oidc/login`, `GET /auth/oidc/callback` | OpenID Connect sign-in |

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
