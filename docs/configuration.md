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
| `database.url` | `FOLIO_DATABASE__URL` | `sqlite:folio.db?mode=rwc` | Database URL. The scheme picks the backend: `sqlite:` or `postgres://` (see [database.md](database.md)). |
| `storage.backend` | `FOLIO_STORAGE__BACKEND` | `local` | `local` or `s3` (see [storage.md](storage.md)). |
| `storage.path` | `FOLIO_STORAGE__PATH` | `./data` | Root directory for the `local` backend. |
| `storage.s3.bucket` | `FOLIO_STORAGE__S3__BUCKET` | unset | Bucket name; required for `s3`. |
| `storage.s3.region` | `FOLIO_STORAGE__S3__REGION` | unset (`us-east-1` for custom endpoints) | Bucket region. |
| `storage.s3.endpoint` | `FOLIO_STORAGE__S3__ENDPOINT` | unset | Endpoint URL for MinIO, Garage, R2 and similar. |
| `storage.s3.access_key_id` | `FOLIO_STORAGE__S3__ACCESS_KEY_ID` | unset | Access key; falls back to the standard `AWS_*` variables and instance credentials. |
| `storage.s3.secret_access_key` | `FOLIO_STORAGE__S3__SECRET_ACCESS_KEY` | unset | Secret key. |
| `storage.s3.path_style` | `FOLIO_STORAGE__S3__PATH_STYLE` | `false` | Address buckets as `endpoint/bucket`; needed for MinIO and Garage. |
| `storage.s3.allow_http` | `FOLIO_STORAGE__S3__ALLOW_HTTP` | `false` | Allow a plain `http://` endpoint. |
| `storage.s3.prefix` | `FOLIO_STORAGE__S3__PREFIX` | unset | Key prefix inside the bucket. |
| `ai.provider` | `FOLIO_AI__PROVIDER` | `openai` | `openai`, `anthropic` or `local`. |
| `ai.openai_api_key` | `FOLIO_AI__OPENAI_API_KEY` | unset | OpenAI API key. |
| `ai.anthropic_api_key` | `FOLIO_AI__ANTHROPIC_API_KEY` | unset | Anthropic API key. |

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
