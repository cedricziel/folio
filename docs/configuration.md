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
| `auth.forward.email_header` | `FOLIO_AUTH__FORWARD__EMAIL_HEADER` | `Remote-Email` | Header carrying the email address. |
| `auth.forward.name_header` | `FOLIO_AUTH__FORWARD__NAME_HEADER` | `Remote-Name` | Header carrying the display name. |
| `auth.forward.groups_header` | `FOLIO_AUTH__FORWARD__GROUPS_HEADER` | `Remote-Groups` | Header carrying group names, separated by `,` or `\|`. |
| `auth.forward.admin_group` | `FOLIO_AUTH__FORWARD__ADMIN_GROUP` | unset | Members of this external group are admins. |
| `auth.oidc.enabled` | `FOLIO_AUTH__OIDC__ENABLED` | `false` | Let users sign in through an OpenID Connect provider. See [auth.md](auth.md#openid-connect). |
| `auth.oidc.issuer` | `FOLIO_AUTH__OIDC__ISSUER` | unset | Issuer URL of the provider, for example `https://auth.example.com/application/o/folio/`. Discovery reads `<issuer>/.well-known/openid-configuration`. |
| `auth.oidc.client_id` | `FOLIO_AUTH__OIDC__CLIENT_ID` | unset | Client ID registered at the provider. |
| `auth.oidc.client_secret` | `FOLIO_AUTH__OIDC__CLIENT_SECRET` | unset | Client secret. Leave unset for a public client. Supports `_FILE`. |
| `auth.oidc.scopes` | `FOLIO_AUTH__OIDC__SCOPES` | `openid profile email` | Space-separated scopes to request. Add one that releases your groups claim if the provider needs it. |
| `auth.oidc.display_name` | `FOLIO_AUTH__OIDC__DISPLAY_NAME` | `Sign in with SSO` | Label of the button on the login page. |
| `auth.oidc.groups_claim` | `FOLIO_AUTH__OIDC__GROUPS_CLAIM` | `groups` | Claim that lists the user's group names. |
| `auth.oidc.admin_group` | `FOLIO_AUTH__OIDC__ADMIN_GROUP` | unset | Members of this external group are admins. |
| `auth.oidc.allow_local_login` | `FOLIO_AUTH__OIDC__ALLOW_LOCAL_LOGIN` | `true` | Keep the username and password form. With `false` only SSO works and `/setup` is not needed. |
| `auth.oidc.auto_create_users` | `FOLIO_AUTH__OIDC__AUTO_CREATE_USERS` | `true` | Create an account on first sign-in. With `false` only users who already signed in once can sign in. |
| `database.url` | `FOLIO_DATABASE__URL` | `sqlite:folio.db?mode=rwc` | Database URL. The scheme picks the backend: `sqlite:` or `postgres://` (see [database.md](database.md)). |
| `storage.backend` | `FOLIO_STORAGE__BACKEND` | `local` | `local` or `s3` (see [storage.md](storage.md)). |
| `storage.path` | `FOLIO_STORAGE__PATH` | `./data` | Root directory for the `local` backend. |
| `storage.s3.bucket` | `FOLIO_STORAGE__S3__BUCKET` | unset | Bucket name; required for `s3`. |
| `storage.s3.region` | `FOLIO_STORAGE__S3__REGION` | unset (`us-east-1` for custom endpoints) | Bucket region. |
| `storage.s3.endpoint` | `FOLIO_STORAGE__S3__ENDPOINT` | unset | Endpoint URL for MinIO, Garage, R2 and similar. |
| `storage.s3.access_key_id` | `FOLIO_STORAGE__S3__ACCESS_KEY_ID` | unset | Access key; falls back to the standard `AWS_*` variables and instance credentials. |
| `storage.s3.secret_access_key` | `FOLIO_STORAGE__S3__SECRET_ACCESS_KEY` | unset | Secret key. |
| `storage.s3.path_style` | `FOLIO_STORAGE__S3__PATH_STYLE` | unset (on when an endpoint is set) | Address buckets as `endpoint/bucket` instead of `bucket.endpoint`. |
| `storage.s3.allow_http` | `FOLIO_STORAGE__S3__ALLOW_HTTP` | `false` | Allow a plain `http://` endpoint. |
| `storage.s3.prefix` | `FOLIO_STORAGE__S3__PREFIX` | unset | Key prefix inside the bucket. |
| `ai.provider` | `FOLIO_AI__PROVIDER` | `openai` | `openai`, `anthropic` or `local`. |
| `ai.openai_api_key` | `FOLIO_AI__OPENAI_API_KEY` | unset | OpenAI API key. |
| `ai.anthropic_api_key` | `FOLIO_AI__ANTHROPIC_API_KEY` | unset | Anthropic API key. |
| `mcp.enabled` | `FOLIO_MCP__ENABLED` | follows `auth.required` | Serve the MCP endpoint at `/mcp`. On when `auth.required` is true, off otherwise because the endpoint is only protected by auth. The `mcp.enabled` database setting overrides it. See [mcp.md](mcp.md). |
| `mcp.allowed_hosts` | `FOLIO_MCP__ALLOWED_HOSTS` | empty | Comma-separated extra `Host` headers `/mcp` accepts besides loopback. |

Ingestion sources have names of your choosing, so they are not listed above:
`FOLIO_INGEST__FOLDERS__<NAME>__<KEY>` and `FOLIO_INGEST__EMAIL__<NAME>__<KEY>`
(for example `FOLIO_INGEST__FOLDERS__INBOX__PATH=/consume`). `_FILE` works on
them too. See [ingestion](ingestion.md).

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
