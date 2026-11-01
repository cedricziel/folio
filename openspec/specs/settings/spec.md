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

The system SHALL store runtime settings as key-value rows in the `settings` table. A migration SHALL seed `ai.provider` (`local`), `ai.openai_api_key`, `ai.anthropic_api_key`, `ai.ocr_model`, `ai.embedding_model`, `storage.path` and the `processing.auto_ocr`, `processing.auto_thumbnail` and `processing.auto_embedding` flags. Of these, `ai.ocr_model`, `ai.embedding_model`, `storage.path` and the `processing.auto_*` flags are stored but not yet applied.

#### Scenario: Fresh database

- **WHEN** Folio starts against an empty database
- **THEN** the settings table contains the seeded keys and the AI provider is `local`

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
