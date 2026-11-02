# settings Specification

## Purpose

How Folio is configured: startup configuration from `config.toml` and the environment, and runtime AI settings stored in the database and edited at `/settings`.
## Requirements
### Requirement: Startup configuration

The system SHALL start without a `config.toml`. Every setting has a built-in default (`127.0.0.1`, `3000`, `sqlite:folio.db?mode=rwc`, `./data` and provider `openai`), `config.toml` (or the file named by `FOLIO_CONFIG`) overrides the defaults, and environment variables override `config.toml`. Settings stored in the database override all of these for the keys the database manages.

#### Scenario: No config file

- **WHEN** Folio starts without a `config.toml` and without `FOLIO_*` variables
- **THEN** it listens on `127.0.0.1:3000` using the default database URL and storage path

#### Scenario: Key missing from a section

- **WHEN** `config.toml` has a `[server]` section without `port`
- **THEN** the server listens on port 3000

### Requirement: Environment overrides

The system SHALL read nested keys from environment variables prefixed `FOLIO_` with a double underscore as the section separator, for example `FOLIO_SERVER__PORT`, `FOLIO_DATABASE__URL`, `FOLIO_STORAGE__PATH`, `FOLIO_AI__PROVIDER`, `FOLIO_AI__OPENAI_API_KEY` and `FOLIO_AI__ANTHROPIC_API_KEY`. The single-underscore names `FOLIO_SERVER_HOST`, `FOLIO_SERVER_PORT`, `FOLIO_DATABASE_URL`, `FOLIO_STORAGE_PATH`, `FOLIO_AI_PROVIDER`, `FOLIO_AI_OPENAI_API_KEY` and `FOLIO_AI_ANTHROPIC_API_KEY` SHALL keep working as deprecated aliases that log a warning, and the double-underscore name SHALL win when both are set. Any variable SHALL also be readable from a file named by the same variable with a `_FILE` suffix; setting both is an error.

#### Scenario: Override the port

- **WHEN** the server starts with `FOLIO_SERVER__PORT=8080`
- **THEN** it listens on port 8080 regardless of `config.toml`

#### Scenario: Override a key that contains an underscore

- **WHEN** the server starts with `FOLIO_AI__OPENAI_API_KEY` set
- **THEN** the value is applied as the OpenAI key fallback

#### Scenario: Secret from a file

- **WHEN** the server starts with `FOLIO_AI__OPENAI_API_KEY_FILE=/run/secrets/openai`
- **THEN** the OpenAI key is the file contents without the trailing newline

#### Scenario: Deprecated alias

- **WHEN** the server starts with `FOLIO_AI_OPENAI_API_KEY` set and no `FOLIO_AI__OPENAI_API_KEY`
- **THEN** the value is applied and a deprecation warning is logged

### Requirement: Database backend

The system SHALL select the database backend from the scheme of `database.url`: `sqlite:` uses SQLite and `postgres://` or `postgresql://` uses PostgreSQL. Each backend SHALL apply its own migrations from `migrations/sqlite` or `migrations/postgres` at startup, and any other scheme SHALL stop startup with an error. On PostgreSQL, semantic search SHALL rank in the database when the pgvector extension is available and fall back to cosine similarity in the application otherwise, and the job queue SHALL claim jobs with `FOR UPDATE SKIP LOCKED`.

#### Scenario: PostgreSQL URL

- **WHEN** Folio starts with `FOLIO_DATABASE__URL=postgres://folio:secret@db/folio`
- **THEN** it applies the PostgreSQL migrations and serves requests from that database

#### Scenario: Unsupported scheme

- **WHEN** Folio starts with `FOLIO_DATABASE__URL=mysql://db/folio`
- **THEN** startup fails with an unsupported database URL error

### Requirement: Database settings

The system SHALL store runtime settings as key-value rows in the `settings` table. Fresh installations SHALL use startup pipeline values without retaining migration-seeded pipeline overrides. Existing installations SHALL preserve every saved row. On upgrade, migrations SHALL copy the legacy `ai.provider`, `ai.ocr_model`, `ai.embedding_model` and `processing.auto_embedding` values into per-stage pipeline keys before removing the legacy keys. `storage.path` and `processing.auto_thumbnail` are stored but not yet applied.

#### Scenario: Fresh database

- **WHEN** Folio starts against an empty database without pipeline configuration
- **THEN** effective `ocr.provider` is `tesseract`, `ocr.languages` is `eng` and `embeddings.provider` is `none`

#### Scenario: Upgrade from OpenAI

- **WHEN** the database had `ai.provider` `openai` and `ai.embedding_model` `text-embedding-3-large`
- **THEN** `ocr.provider` is `openai-vision`, `embeddings.provider` is `openai` and `embeddings.model` is `text-embedding-3-large`

### Requirement: AI settings page

The system SHALL show the AI settings at `GET /settings` and save them at `POST /settings/ai`. The form SHALL offer the providers `local`, `openai` and `anthropic`, the two API keys, and the OCR and embedding model names, and the save SHALL respond with a Turbo Stream confirmation.

#### Scenario: Switch to OpenAI

- **WHEN** a user selects `openai`, enters an API key and saves
- **THEN** the `ai.provider` and `ai.openai_api_key` rows hold the new values

### Requirement: Effective AI settings

The OCR and embedding services SHALL read the provider and API keys from the database, cached for up to 5 seconds. The database provider SHALL win over `config.toml`; an API key that is empty in the database SHALL fall back to the value from `config.toml`.

#### Scenario: Change takes effect without a restart

- **WHEN** a user saves a new provider on `/settings`
- **THEN** OCR and search use it within 5 seconds, without restarting the server

#### Scenario: Key only in config.toml

- **WHEN** the database OpenAI key is empty and `config.toml` sets `ai.openai_api_key`
- **THEN** OpenAI requests use the key from `config.toml`

#### Scenario: Local provider

- **WHEN** the provider is `local`
- **THEN** no AI provider is called for OCR or embeddings

### Requirement: Pipeline startup configuration

The system SHALL accept `ocr.provider`, `ocr.model`, `ocr.languages`, `ocr.base_url`, `embeddings.provider`, `embeddings.model`, `embeddings.base_url`, and `processing.auto_ocr`, `processing.auto_thumbnail`, `processing.auto_embed` through typed startup configuration and corresponding `FOLIO_` environment variables. Precedence SHALL be built-in defaults, config file, environment, then saved database rows. The HTML settings page SHALL show the effective pipeline values.

#### Scenario: Environment overrides a partial file

- **WHEN** a config file selects an OCR provider and language and the environment selects another provider
- **THEN** the environment provider and configured language are used and missing fields retain built-in defaults

#### Scenario: Saved settings win

- **WHEN** a database row overrides a configured provider, model, or processing toggle
- **THEN** that row wins while missing database keys retain startup values

#### Scenario: Fresh installation

- **WHEN** a new database is initialized with OCR disabled and embeddings unconfigured through environment variables
- **THEN** migration-seeded defaults do not hide those values and no OCR or embedding provider is called

#### Scenario: Existing installation

- **WHEN** an existing database is initialized with different startup values
- **THEN** every saved pipeline row remains unchanged, including values equal to built-in defaults

#### Scenario: Interrupted initialization

- **WHEN** a fresh installation restarts after migrations finish but before pipeline defaults are removed
- **THEN** initialization retries cleanup and records completion atomically, so seeded rows cannot hide startup values
