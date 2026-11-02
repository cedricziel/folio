## ADDED Requirements

### Requirement: Bounded document browsing

The system SHALL support vault-scoped paged document queries with chronological or title ordering, stable ID tie-breakers, existing classification/date filters, and limits from 1 through 100. Legacy list callers SHALL retain their existing behavior.

#### Scenario: Equal primary sort values
- **WHEN** several matching documents have identical timestamps or titles and the client requests successive pages
- **THEN** every matching document is returned exactly once while the data remains unchanged

#### Scenario: Query-bound continuation
- **WHEN** a continuation is reused with another vault, filter, or sort
- **THEN** the query is rejected rather than returning documents from a different result set

### Requirement: Authorized paged API

After the vault-access dependency lands, `GET /api/vaults/{vault_id}/documents` SHALL expose bounded browse queries with enriched summaries and an optional next cursor.

#### Scenario: Successful page
- **WHEN** an authorized client requests a valid page
- **THEN** the API returns 200 with at most the requested number of documents and a continuation only when more matches exist

#### Scenario: Invalid page
- **WHEN** a client supplies an invalid limit or mismatched cursor
- **THEN** the API returns 400

#### Scenario: Inaccessible vault
- **WHEN** a signed-in caller has no access to the requested vault
- **THEN** the API returns 404 without document metadata

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
