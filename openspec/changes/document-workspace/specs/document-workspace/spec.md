## ADDED Requirements

### Requirement: Bounded document browsing

The system SHALL support vault-scoped paged document queries with chronological or title ordering, stable ID tie-breakers, existing classification/date filters, and limits from 1 through 100. Current HTML and JSON list routes SHALL remain supported.

#### Scenario: Equal primary sort values
- **WHEN** several matching documents have identical timestamps or titles and the client requests successive pages
- **THEN** every matching document is returned exactly once while the data remains unchanged

#### Scenario: Query-bound continuation
- **WHEN** a continuation is reused with another vault, filter, or sort
- **THEN** the query is rejected rather than returning documents from a different result set

### Requirement: Authorized paged API

`GET /api/vaults/{vault_id}/documents` SHALL expose bounded browse queries with enriched summaries and an optional next cursor. Each request SHALL check the existing vault access policy. `GET /api/documents/browse` SHALL provide the same contract for the resolved default vault, without changing legacy list routes. Unauthenticated requests, permitted only when authentication is disabled, SHALL browse only the instance default vault.

#### Scenario: Successful page
- **WHEN** an authorized client requests a valid page
- **THEN** the API returns 200 with at most the requested number of documents and a continuation only when more matches exist

#### Scenario: Invalid page
- **WHEN** a client supplies an invalid limit or mismatched cursor
- **THEN** the API returns 400

#### Scenario: Inaccessible vault
- **WHEN** a signed-in caller has no access to the requested vault
- **THEN** the API returns 404 without document metadata

### Requirement: Literal search with filters

Paged browse requests SHALL accept `q` as a literal text query. Queries with more than 32 whitespace-separated terms SHALL return 400 before querying the document repository. Every term SHALL match the title or content. Text and all selected classification/date filters SHALL apply before pagination, without requiring an AI provider. Existing hybrid search SHALL remain unchanged.

#### Scenario: Match outside the first unfiltered page
- **WHEN** a document outside the first unfiltered page matches the text and every selected tag
- **THEN** it appears in the filtered page and a document missing any selected tag does not

### Requirement: Shared browser state

The native app SHALL provide Library navigation and preserve query and selection while opening and closing document details. List and grid SHALL share query state when grid support lands.

#### Scenario: Return from detail
- **WHEN** a user opens a result and returns to the browser
- **THEN** the destination, query, and selection remain unchanged

### Requirement: Explicit review completion

The system SHALL store review completion separately from processing and classification, preserve legacy Filed membership during migration, and keep new imports unreviewed until explicitly reviewed.

#### Scenario: Automatic classification
- **WHEN** a processing workflow assigns a correspondent to a new import
- **THEN** the document remains unreviewed

#### Scenario: Reprocessing
- **WHEN** a reviewed document is reprocessed
- **THEN** its review decision is preserved and processing failures remain discoverable
