# settings Specification

## Purpose

How Folio is configured: startup configuration from `config.toml` and the environment, and runtime AI settings stored in the database and edited at `/settings`.

## Requirements

### Requirement: Startup configuration

The system SHALL read `config.toml` at startup and let environment variables prefixed `FOLIO_` override it. It SHALL configure the server host and port, the database URL, the storage path, and fallback AI settings. Each of the `[server]`, `[database]`, `[storage]` and `[ai]` sections must be present; keys missing inside a section default to `127.0.0.1`, `3000`, `sqlite:folio.db?mode=rwc`, `./data` and provider `openai`.

#### Scenario: Override the port

- **WHEN** the server starts with `FOLIO_SERVER_PORT=8080`
- **THEN** it listens on port 8080 regardless of `config.toml`

#### Scenario: Key missing from a section

- **WHEN** `config.toml` has a `[server]` section without `port`
- **THEN** the server listens on port 3000

### Requirement: Database settings

The system SHALL store runtime settings as key-value rows in the `settings` table. A migration SHALL seed `ai.provider` (`local`), `ai.openai_api_key`, `ai.anthropic_api_key`, `ai.ocr_model`, `ai.embedding_model`, `storage.path` and the `processing.auto_ocr`, `processing.auto_thumbnail` and `processing.auto_embedding` flags.

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
