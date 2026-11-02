## MODIFIED Requirements

### Requirement: Database settings

The system SHALL store runtime settings as key-value rows in the `settings` table. Fresh installations SHALL use startup pipeline values without retaining migration-seeded pipeline overrides. Existing installations SHALL preserve every saved row. On upgrade, migrations SHALL copy the legacy `ai.provider`, `ai.ocr_model`, `ai.embedding_model` and `processing.auto_embedding` values into per-stage pipeline keys before removing the legacy keys. `storage.path` and `processing.auto_thumbnail` are stored but not yet applied.

#### Scenario: Fresh database

- **WHEN** Folio starts against an empty database without pipeline configuration
- **THEN** effective `ocr.provider` is `tesseract`, `ocr.languages` is `eng` and `embeddings.provider` is `none`

#### Scenario: Upgrade from OpenAI

- **WHEN** the database had `ai.provider` `openai` and `ai.embedding_model` `text-embedding-3-large`
- **THEN** `ocr.provider` is `openai-vision`, `embeddings.provider` is `openai` and `embeddings.model` is `text-embedding-3-large`

### Requirement: AI settings page

The system SHALL show the settings at `GET /settings`. `POST /settings/ai` SHALL save the two API keys and respond with a Turbo Stream confirmation. `POST /settings/processing` SHALL save the OCR and embedding provider, model, languages and base URL and the two auto flags, then redirect to `/settings`. `POST /settings/processing/test` SHALL build the providers from the submitted, unsaved values and render the result of their health checks. `POST /settings/processing/reembed` SHALL mark vectors of other models stale, reset `embed_status` for documents with text, enqueue an embedding job for each and redirect to `/settings`.

#### Scenario: Switch to OpenAI

- **WHEN** a user selects `openai-vision` and `openai`, enters an API key and saves
- **THEN** the `ocr.provider`, `embeddings.provider` and `ai.openai_api_key` rows hold the new values

#### Scenario: Switch to Ollama embeddings

- **WHEN** a user selects `ollama`, sets a model and saves
- **THEN** the `embeddings.provider` and `embeddings.model` rows hold the new values

#### Scenario: Test connection

- **WHEN** a user presses Test connection with an unreachable base URL
- **THEN** the page shows the failure for that stage without saving anything

#### Scenario: Re-embed all

- **WHEN** a user presses Re-embed all
- **THEN** every document with text gets a new embedding job

### Requirement: Effective AI settings

The OCR and embedding services SHALL read provider, model, language and base URL settings from the database, cached for up to 5 seconds and invalidated when settings are saved. The database SHALL win over `config.toml`; an API key that is empty in the database SHALL fall back to the value from `config.toml`. A provider SHALL be rebuilt only when its settings change.

#### Scenario: Change takes effect without a restart

- **WHEN** a user saves a new provider on `/settings`
- **THEN** OCR and search use it immediately, without restarting the server

#### Scenario: Key only in config.toml

- **WHEN** the database OpenAI key is empty and `config.toml` sets `ai.openai_api_key`
- **THEN** OpenAI requests use the key from `config.toml`

#### Scenario: Local provider

- **WHEN** `ocr.provider` is `tesseract` and `embeddings.provider` is `none`
- **THEN** no hosted AI provider is called for OCR or embeddings

#### Scenario: Disabled providers

- **WHEN** the OCR and embedding providers are `none`
- **THEN** no AI provider is called for OCR or embeddings

#### Scenario: Unknown provider id

- **WHEN** a setting names a provider that is not registered
- **THEN** the stage fails with an error listing the available providers
