# Spec Delta

## MODIFIED Requirements

### Requirement: Semantic ranking

The system SHALL embed the query, score the stored embedding of every document in the searched vault by cosine similarity to it, and return the highest-scoring documents first, up to a limit. Only documents of that vault with a stored embedding SHALL be returned.

#### Scenario: Relevant document ranks first

- **WHEN** the provider is `openai` with a valid key and the query is close in meaning to one document's text in the searched vault
- **THEN** that document is returned first with the highest score

#### Scenario: Better match in another vault

- **WHEN** a user searches `Home` and the closest document to the query is in `Business`
- **THEN** the `Business` document is not returned, even if the user is a member of `Business`

#### Scenario: Local provider

- **WHEN** the provider is `local` or `anthropic`
- **THEN** the query embedding is a zero vector and every result scores 0

### Requirement: JSON search API

`GET /api/vaults/{vault_id}/documents/search?q=<query>&limit=<n>` and `POST /api/vaults/{vault_id}/documents/search` with `{"query": ..., "limit": ...}` SHALL return `{"results": [{"document": ..., "score": ...}]}` with status 200 to any member of the vault. The limit SHALL default to 10.

#### Scenario: Search with the default limit

- **WHEN** a viewer of `Home` requests `GET /api/vaults/{home}/documents/search?q=insurance`
- **THEN** the response status is 200 and at most 10 `Home` documents are returned

#### Scenario: Embedding fails

- **WHEN** the provider is `openai` and no API key is configured
- **THEN** the response status is 500

#### Scenario: Vault without a role

- **WHEN** a user with no role in `Home` requests `GET /api/vaults/{home}/documents/search?q=insurance`
- **THEN** the response status is 404

#### Scenario: Former unscoped path

- **WHEN** a client requests `GET /api/documents/search?q=insurance`
- **THEN** the response status is 404

### Requirement: Search in the web UI

`GET /vaults/{vault_id}/documents/search?q=<query>` SHALL return an HTML fragment with up to 10 of the vault's documents ranked by similarity, each score shown as a percentage with one decimal place. When the query cannot be embedded, the system SHALL instead list every document of the vault whose title contains the query (case-insensitive), with no limit, scoring each 100.0.

#### Scenario: Search with a working provider

- **WHEN** a user searches `Home` and the query embeds successfully
- **THEN** the fragment lists up to 10 `Home` documents ranked by similarity

#### Scenario: Provider unconfigured

- **WHEN** the provider is `openai`, no API key is configured, and a user searches `Home` for `tax`
- **THEN** every `Home` document whose title contains `tax` in any case is listed with score 100.0, and no document from another vault
