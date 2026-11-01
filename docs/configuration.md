# Configuration

Folio is configured through environment variables first. A `config.toml` is
optional; every setting has a built-in default.

## Precedence

From lowest to highest:

1. Built-in defaults
2. `config.toml` (path from `FOLIO_CONFIG`, default `./config.toml`; a missing file is fine)
3. Environment variables
4. Settings stored in the database (managed on the `/settings` page)

The database overrides the other layers only for the keys it manages:
`ai.provider`, `ai.openai_api_key`, `ai.anthropic_api_key`, `ai.ocr_model`,
`ai.embedding_model` and the `processing.*` toggles. Everything else is fixed at
startup by the first three layers.

## Environment variable naming

- Prefix `FOLIO_`.
- Nested keys are separated by a double underscore: `ai.openai_api_key` is
  `FOLIO_AI__OPENAI_API_KEY`. Single underscores stay part of the key name.
- Any variable accepts a `_FILE` companion that names a file holding the value,
  for Docker and Kubernetes secrets: `FOLIO_AI__OPENAI_API_KEY_FILE=/run/secrets/openai`.
  A trailing newline is stripped. Setting both the variable and its `_FILE`
  companion is an error.
- `.env` files in the working directory are loaded for local development.

## Settings

| `config.toml` key | Environment variable | Default | Description |
| --- | --- | --- | --- |
| `server.host` | `FOLIO_SERVER__HOST` | `127.0.0.1` | Address to bind. Use `0.0.0.0` in containers. |
| `server.port` | `FOLIO_SERVER__PORT` | `3000` | TCP port to listen on. |
| `server.public_url` | `FOLIO_SERVER__PUBLIC_URL` | unset | Externally visible base URL, used in OAuth metadata, redirects and links in webhook payloads. Derived from the request when unset. |
| `auth.required` | `FOLIO_AUTH__REQUIRED` | `true` | Require a login or token for the web UI and API. See [auth](auth.md). |
| `auth.forward.enabled` | `FOLIO_AUTH__FORWARD__ENABLED` | `false` | Trust identity headers from a reverse proxy or SSO gateway. See [auth.md](auth.md#forward-auth). |
| `auth.forward.trusted_proxies` | `FOLIO_AUTH__FORWARD__TRUSTED_PROXIES` | empty | Comma-separated CIDRs or addresses of the proxies whose headers are honoured. Headers from any other peer are ignored. |
| `auth.forward.user_header` | `FOLIO_AUTH__FORWARD__USER_HEADER` | `Remote-User` | Header carrying the username. |
| `auth.forward.user_id_header` | `FOLIO_AUTH__FORWARD__USER_ID_HEADER` | empty | Header carrying a stable account id (Pangolin: `Remote-User-Id`). When set, accounts are linked by it and `user_header` is only the display username; requests without it are unauthenticated. |
| `auth.forward.email_header` | `FOLIO_AUTH__FORWARD__EMAIL_HEADER` | `Remote-Email` | Header carrying the email address. |
| `auth.forward.name_header` | `FOLIO_AUTH__FORWARD__NAME_HEADER` | `Remote-Name` | Header carrying the display name. |
| `auth.forward.groups_header` | `FOLIO_AUTH__FORWARD__GROUPS_HEADER` | empty | Header carrying group names. Empty means groups are ignored. |
| `auth.forward.group_separator` | `FOLIO_AUTH__FORWARD__GROUP_SEPARATOR` | `,` | Separator between group names in `groups_header` (Authentik: `\|`). |
| `auth.forward.role_header` | `FOLIO_AUTH__FORWARD__ROLE_HEADER` | empty | Header carrying a single role name (Pangolin: `Remote-Role`). Empty means roles are ignored. |
| `auth.forward.admin_group` | `FOLIO_AUTH__FORWARD__ADMIN_GROUP` | unset | Members of this external group are admins. Needs `groups_header`. |
| `auth.forward.shared_secret` | `FOLIO_AUTH__FORWARD__SHARED_SECRET` | unset | When set, the proxy must also send this value in `secret_header`; requests without it are treated as unauthenticated. |
| `auth.forward.secret_header` | `FOLIO_AUTH__FORWARD__SECRET_HEADER` | `X-Folio-Proxy-Secret` | Header carrying the shared secret. |
| `auth.forward.session_ttl_secs` | `FOLIO_AUTH__FORWARD__SESSION_TTL_SECS` | `43200` | Lifetime of sessions created by forward-auth logins (12 hours). Local logins last 14 days. |
| `auth.forward.admin_role` | `FOLIO_AUTH__FORWARD__ADMIN_ROLE` | unset | Users with this role are admins. Needs `role_header`. |
| `database.url` | `FOLIO_DATABASE__URL` | `sqlite:folio.db?mode=rwc` | Database URL. The scheme picks the backend: `sqlite:` or `postgres://` (see [database.md](database.md)). |
| `storage.path` | `FOLIO_STORAGE__PATH` | `./data` | Directory for stored documents and thumbnails. |
| `ai.provider` | `FOLIO_AI__PROVIDER` | `openai` | `openai`, `anthropic` or `local`. |
| `ai.openai_api_key` | `FOLIO_AI__OPENAI_API_KEY` | unset | OpenAI API key. |
| `ai.anthropic_api_key` | `FOLIO_AI__ANTHROPIC_API_KEY` | unset | Anthropic API key. |
| `mcp.enabled` | `FOLIO_MCP__ENABLED` | follows `auth.required` | Serve the MCP endpoint at `/mcp`. On when `auth.required` is true, off otherwise because the endpoint is only protected by auth. The `mcp.enabled` database setting overrides it. See [mcp.md](mcp.md). |
| `mcp.allowed_hosts` | `FOLIO_MCP__ALLOWED_HOSTS` | empty | Comma-separated extra `Host` headers `/mcp` accepts besides loopback. |

Other variables read directly: `FOLIO_CONFIG` (config file path) and `RUST_LOG`
(log filter).

## Deprecated names

These single-underscore names still work and log a deprecation warning when
used. When both forms are set, the new name wins.

| Deprecated | Use instead |
| --- | --- |
| `FOLIO_SERVER_HOST` | `FOLIO_SERVER__HOST` |
| `FOLIO_SERVER_PORT` | `FOLIO_SERVER__PORT` |
| `FOLIO_DATABASE_URL` | `FOLIO_DATABASE__URL` |
| `FOLIO_STORAGE_PATH` | `FOLIO_STORAGE__PATH` |
| `FOLIO_AI_PROVIDER` | `FOLIO_AI__PROVIDER` |
| `FOLIO_AI_OPENAI_API_KEY` | `FOLIO_AI__OPENAI_API_KEY` |
| `FOLIO_AI_ANTHROPIC_API_KEY` | `FOLIO_AI__ANTHROPIC_API_KEY` |
