## ADDED Requirements

### Requirement: Durable original and archival identity
The system SHALL record actual SHA-256, byte size, MIME type, and storage key for every new original and retained archival derivative. Published bytes SHALL be immutable. Legacy bytes SHALL be verified through a restartable backfill with explicit pending/error states.

#### Scenario: All import paths preserve bytes
- **WHEN** web, JSON, MCP, folder, email, or scanner ingestion finalizes an original
- **THEN** its recorded checksum and size match the stored bytes and later processing cannot replace those bytes

#### Scenario: Missing legacy original
- **WHEN** backfill cannot read a legacy original
- **THEN** its first revision remains explicitly unavailable and retryable without fabricated identity or replacement bytes

### Requirement: Stable documents with immutable numbered revisions
A document SHALL retain its logical ID and metadata across file replacement. Revisions SHALL be immutable, monotonically numbered, vault-owned history. POST `/api/documents/{id}/revisions` SHALL require expected current identity and an idempotency key.

#### Scenario: Concurrent replacements
- **WHEN** two authorized POST `/api/documents/{id}/revisions` requests replace the same expected current revision
- **THEN** one returns 201 and the other returns 409
- **AND** previous bytes remain unchanged and no two revisions share a number

#### Scenario: Retry a committed replacement
- **WHEN** POST `/api/documents/{id}/revisions` repeats a committed idempotency key with identical bytes and request identity
- **THEN** the response is 200 with the existing revision and creates no duplicate history

### Requirement: Historical download and restoration
GET `/api/documents/{id}/revisions` SHALL return bounded history with status 200 for authorized readers. GET `/api/documents/{id}/revisions/{revision_id}/download` SHALL return the historical original with status 200. POST `/api/documents/{id}/revisions/{revision_id}/restore` SHALL create a new current revision, preserving earlier history.

#### Scenario: Restore old bytes
- **WHEN** an authorized editor posts a valid conditional restore request to `/api/documents/{id}/revisions/{revision_id}/restore`
- **THEN** the response is 201 with a new revision number, source provenance, and server-derived actor
- **AND** logical metadata and links remain attached while review is cleared

#### Scenario: Cross-vault history access
- **WHEN** an unauthorized caller requests GET `/api/documents/{id}/revisions/{revision_id}/download`
- **THEN** the response is 404 and discloses no original metadata or bytes

### Requirement: Revision-bound processing
OCR, chunks, previews, and retained archival outputs SHALL identify their source revision. Stale work SHALL NOT change the current document projection or current search results.

#### Scenario: Late OCR completion
- **WHEN** OCR for revision 1 completes after revision 2 becomes current with AI local or unconfigured
- **THEN** revision 2 retains its own processing state and search content

### Requirement: Whole-document lifecycle
Trash, restore, purge, and vault moves SHALL apply to the complete revision history. Purge SHALL remove owned objects and fence late writes. Moves SHALL preserve history IDs and update ownership atomically.

#### Scenario: Move with active processing
- **WHEN** a document moves to another authorized vault during a revision job
- **THEN** source-vault reads lose access and a stale job cannot write under the old ownership

### Requirement: Web and Apple file history
Both clients SHALL provide current file details, revision history, replacement, historical download, and restore-as-new controls with conflict and retry states.

#### Scenario: Conflict in either client
- **WHEN** another editor replaces the file before the user's replacement commits
- **THEN** the client shows the conflict and refreshes current revision state without silently retrying over the other editor's revision

### Requirement: Committed workflow events
The system SHALL publish domain-specific transitions through the existing durable outbox using the foundation's versioned, actor-attributed, vault-scoped envelope. Event identity SHALL survive delivery retries. Both workflow editors SHALL expose understandable trigger labels and supported condition fields. Mutations SHALL emit no event on rollback, unchanged state, or idempotent retry. Workflow-origin transitions SHALL retain recursion protection.

#### Scenario: Commit and retry
- **WHEN** a document transition commits and its request or delivery is retried
- **THEN** one event per applicable transition remains in the outbox with its original identity, occurrence time, actor, and immutable condition data

#### Scenario: Moved or deleted subject
- **WHEN** an event is consumed after the logical document moves or is deleted
- **THEN** workflow actions recheck subject access and cannot use the snapshot to access the old vault or resurrect deleted content
