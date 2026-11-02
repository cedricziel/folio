## ADDED Requirements

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

## MODIFIED Requirements

### Requirement: Database settings

The system SHALL store runtime settings as key-value rows in the `settings` table. Fresh installations SHALL use startup pipeline values without retaining migration-seeded pipeline overrides. Existing installations SHALL preserve every saved row. On upgrade, migrations SHALL copy the legacy `ai.provider`, `ai.ocr_model`, `ai.embedding_model` and `processing.auto_embedding` values into per-stage pipeline keys before removing the legacy keys. `storage.path` and `processing.auto_thumbnail` are stored but not yet applied.

#### Scenario: Fresh database

- **WHEN** Folio starts against an empty database without pipeline configuration
- **THEN** effective `ocr.provider` is `tesseract`, `ocr.languages` is `eng` and `embeddings.provider` is `none`

#### Scenario: Upgrade from OpenAI

- **WHEN** the database had `ai.provider` `openai` and `ai.embedding_model` `text-embedding-3-large`
- **THEN** `ocr.provider` is `openai-vision`, `embeddings.provider` is `openai` and `embeddings.model` is `text-embedding-3-large`
