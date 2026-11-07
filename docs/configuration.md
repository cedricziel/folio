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
`ai.openai_api_key`, `ai.anthropic_api_key`, `ocr.*`, `embeddings.*`,
and the `processing.*` toggles. Everything else is fixed at
startup by the first three layers.

New databases use startup pipeline values until settings are saved. Upgrades
preserve existing database rows, including values seeded by earlier releases.

## Environment variable naming

- Prefix `FOLIO_`.
- Nested keys are separated by a double underscore: `ai.openai_api_key` is
  `FOLIO_AI__OPENAI_API_KEY`. Single underscores stay part of the key name.
  Names such as `FOLIO_AI_OPENAI_API_KEY` are ignored.
- Any variable accepts a `_FILE` companion that names a file holding the value,
  for Docker and Kubernetes secrets: `FOLIO_AI__OPENAI_API_KEY_FILE=/run/secrets/openai`.
  A trailing newline is stripped. Setting both the variable and its `_FILE`
  companion is an error.
- `.env` files in the working directory are loaded for local development.

## Settings

| `config.toml` key | Environment variable | Default | Description |
| --- | --- | --- | --- |
| `ocr.provider` | `FOLIO_OCR__PROVIDER` | `tesseract` | OCR provider; see [pipeline](pipeline.md). |
| `ocr.model` | `FOLIO_OCR__MODEL` | `empty` | Vision model; empty selects the provider default. |
| `ocr.languages` | `FOLIO_OCR__LANGUAGES` | `eng` | OCR languages joined with `+`, such as `deu+eng`. |
| `ocr.base_url` | `FOLIO_OCR__BASE_URL` | `empty` | OCR endpoint override. |
| `embeddings.provider` | `FOLIO_EMBEDDINGS__PROVIDER` | `none` | Embedding provider; `none` disables semantic search. |
| `embeddings.model` | `FOLIO_EMBEDDINGS__MODEL` | `empty` | Embedding model; empty selects the provider default. |
| `embeddings.base_url` | `FOLIO_EMBEDDINGS__BASE_URL` | `empty` | Embedding endpoint override. |
| `processing.auto_ocr` | `FOLIO_PROCESSING__AUTO_OCR` | `true` | Run OCR automatically. |
| `processing.auto_thumbnail` | `FOLIO_PROCESSING__AUTO_THUMBNAIL` | `true` | Stored thumbnail preference; the worker currently generates thumbnails regardless. |
| `processing.auto_embed` | `FOLIO_PROCESSING__AUTO_EMBED` | `true` | Generate embeddings automatically when a provider is configured. |
| `server.host` | `FOLIO_SERVER__HOST` | `127.0.0.1` | Address to bind. Use `0.0.0.0` in containers. |
| `server.port` | `FOLIO_SERVER__PORT` | `3000` | TCP port to listen on. |
| `server.public_url` | `FOLIO_SERVER__PUBLIC_URL` | unset | Externally visible base URL, used in OAuth metadata, redirects and links in webhook payloads. Derived from the request when unset. |
| `auth.required` | `FOLIO_AUTH__REQUIRED` | `true` | Require a login or token for the web UI and API. See [auth](auth.md). |
| `auth.setup_token` | `FOLIO_AUTH__SETUP_TOKEN` | random per start | One-time secret that `/setup` requires while no user exists. When unset Folio generates one and logs the full setup URL at startup. Supports `FOLIO_AUTH__SETUP_TOKEN_FILE`. See [auth](auth.md). |
| `auth.secret_key` | `FOLIO_AUTH__SECRET_KEY` | generated on first start | Key that encrypts two-factor secrets in the database (at least 16 characters; `openssl rand -base64 32` works). When unset Folio generates one, stores it in the database settings and warns at startup. Set it yourself so a copy of the database alone cannot decrypt them. Changing it makes enrolled authenticator apps unusable. Supports `FOLIO_AUTH__SECRET_KEY_FILE`. See [auth](auth.md#two-factor-sign-in). |
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
| `import.allowed_roots` | `FOLIO_IMPORT__ALLOWED_ROOTS` | empty | Comma-separated server directories an import may read from. Empty turns imports off. Config only, not on `/settings`. See [paperless-import.md](paperless-import.md). |
| `import.max_file_bytes` | `FOLIO_IMPORT__MAX_FILE_BYTES` | `536870912` (512 MB) | Largest original file an import accepts; bigger files fail with `file_too_large`. |

Ingestion sources have names of your choosing, so they are not listed above:
`FOLIO_INGEST__FOLDERS__<NAME>__<KEY>` and `FOLIO_INGEST__EMAIL__<NAME>__<KEY>`
(for example `FOLIO_INGEST__FOLDERS__INBOX__PATH=/consume`). `_FILE` works on
them too. See [ingestion](ingestion.md).

Other variables read directly: `FOLIO_CONFIG` (config file path) and `RUST_LOG`
(log filter).
