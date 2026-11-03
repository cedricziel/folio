## ADDED Requirements

### Requirement: Durable item outcomes
The system SHALL publish imported, skipped, duplicate, and failed outcomes from committed state transitions through the shared domain outbox. Each outcome SHALL retain stable item and attempt identity, resolved vault, occurrence time, and nullable server-derived actor.

#### Scenario: Completion is replayed
- **WHEN** an already committed attempt completion is replayed after a crash
- **THEN** no additional outcome or event is created

#### Scenario: Publication fails
- **WHEN** the transactional outbox append fails
- **THEN** outcome persistence and terminal state transition roll back together

#### Scenario: Unchanged item is polled again
- **WHEN** a folder item or email attachment with an existing completed observation is seen again
- **THEN** it does not start another attempt or emit another duplicate outcome

#### Scenario: An import fails and retries
- **WHEN** a failed import is retried under a new fenced claim
- **THEN** it retains the item identity, creates a new attempt identity, and cannot be completed by the stale attempt

### Requirement: Preserve synchronous import veto
The system SHALL evaluate SkipImport synchronously before importing bytes. Durable outcome workflows SHALL NOT claim to veto or undo an import.

#### Scenario: Rule skips an item
- **WHEN** an enabled ingest.received rule matches SkipImport
- **THEN** no document is imported and a committed skipped outcome has no document ID

#### Scenario: Rule changes after a skip
- **WHEN** the same item is evaluated after a relevant rule change
- **THEN** a previous skipped observation does not permanently prevent admission

### Requirement: Source health transitions
The system SHALL publish source.failed on entry into unavailable state and source.recovered on recovery. Source status and events SHALL commit together with resolved vault ownership.

#### Scenario: Source remains unavailable
- **WHEN** repeated polls fail while the source is already unavailable
- **THEN** no additional source.failed event is published

#### Scenario: Source recovers
- **WHEN** an unavailable source completes a successful poll
- **THEN** exactly one source.recovered event is committed with the healthy status

### Requirement: Bounded outcome visibility
Outcome payloads SHALL contain only allowlisted identifiers and bounded codes. Web and Apple source views SHALL show readable recent outcomes and health, with loading, empty, retry, and failed-load states. Shared workflow builders SHALL expose readable triggers and applicable conditions without allowing incompatible actions.

#### Scenario: Email error includes private data
- **WHEN** an email provider returns an error containing a credential or raw message data
- **THEN** neither the durable event nor outcome UI contains that data

#### Scenario: Non-document outcome is selected
- **WHEN** a user selects a skipped, duplicate, failed, or source-health trigger
- **THEN** both builders reject document-only actions and SkipImport with a readable explanation

#### Scenario: Imported document is no longer accessible
- **WHEN** a source outcome references a document the current user cannot access
- **THEN** the UI does not expose document details and workflow actions recheck access before execution

### Requirement: Read source outcome history
The JSON API SHALL provide `GET /api/sources/{id}/outcomes`, scoped by the existing `X-Folio-Vault` header or authenticated default vault. It SHALL require source-management access, return bounded pages ordered by descending outcome ID, and hide document links when the document is no longer readable. The HTML route SHALL be `GET /settings/sources/{id}/outcomes`, with the existing vault prefix supported.

#### Scenario: Source has no outcomes
- **WHEN** an authorized owner requests an existing source's outcome history
- **THEN** the JSON API returns 200 with vault_id, source_id, an empty outcomes array, and null next_before
- **AND** the HTML page displays an empty-state explanation

#### Scenario: Invalid page request
- **WHEN** before is nonpositive or limit is outside 1 through 100
- **THEN** the JSON API returns 400

#### Scenario: Source is missing or belongs to another vault
- **WHEN** an authorized owner requests a source outside the selected vault
- **THEN** the JSON API returns 404 without outcome data

#### Scenario: User cannot manage sources
- **WHEN** a vault member without source-management permission requests outcomes
- **THEN** the API returns 403 and the HTML page does not expose source history

#### Scenario: Outcome storage is unavailable
- **WHEN** a history read fails
- **THEN** the JSON API returns 500 and the HTML page offers retry without exposing the storage error
