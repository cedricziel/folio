# document-search Specification

## Purpose

Finding documents by meaning: queries are embedded with the configured AI provider and compared against stored document embeddings.

## Requirements

### Requirement: Semantic ranking

The system SHALL embed the query, score every stored document embedding by cosine similarity to it, and return the highest-scoring documents first, up to a limit. Only documents with a stored embedding SHALL be returned.

#### Scenario: Relevant document ranks first

- **WHEN** the provider is `openai` with a valid key and the query is close in meaning to one document's text
- **THEN** that document is returned first with the highest score

#### Scenario: Local provider

- **WHEN** the provider is `local` or `anthropic`
- **THEN** the query embedding is a zero vector and every result scores 0

### Requirement: JSON search API

`GET /api/documents/search?q=<query>&limit=<n>` and `POST /api/documents/search` with `{"query": ..., "limit": ...}` SHALL return `{"results": [{"document": ..., "score": ...}]}` with status 200. The limit SHALL default to 10.

#### Scenario: Search with the default limit

- **WHEN** a client requests `GET /api/documents/search?q=insurance`
- **THEN** the response status is 200 and at most 10 results are returned

#### Scenario: Embedding fails

- **WHEN** the provider is `openai` and no API key is configured
- **THEN** the response status is 500

### Requirement: Search in the web UI

`GET /documents/search?q=<query>` SHALL return an HTML fragment with up to 10 results ranked by similarity, each score shown as a percentage with one decimal place. When the query cannot be embedded, the system SHALL instead list every document whose title contains the query (case-insensitive), with no limit, scoring each 100.0.

#### Scenario: Search with a working provider

- **WHEN** a user searches and the query embeds successfully
- **THEN** the fragment lists up to 10 documents ranked by similarity

#### Scenario: Provider unconfigured

- **WHEN** the provider is `openai`, no API key is configured, and a user searches for `tax`
- **THEN** every document whose title contains `tax` in any case is listed with score 100.0
