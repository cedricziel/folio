## ADDED Requirements

### Requirement: Durable processing outcomes
The system SHALL commit stage state and its versioned domain event in the same transaction on SQLite and PostgreSQL.

#### Scenario: Transaction rollback
- **WHEN** event insertion fails during stage completion
- **THEN** stage state and result references remain unchanged and no completion event is visible

#### Scenario: Duplicate or stale completion
- **WHEN** completion repeats or its lease, revision, document lifecycle, or vault fence is stale
- **THEN** it produces no state change or additional event

### Requirement: Meaningful retry and skip states
The system SHALL distinguish retrying attempts from terminal stage failure.

#### Scenario: Retryable provider failure
- **WHEN** a provider attempt fails and retry budget remains
- **THEN** the state and event describe a scheduled retry with a sanitized reason, not terminal failure

#### Scenario: Unconfigured provider
- **WHEN** OCR or embedding uses an unconfigured or disabled provider
- **THEN** the stage commits a skipped outcome with provider_disabled

#### Scenario: Empty OCR text
- **WHEN** OCR successfully returns no text
- **THEN** OCR completes once and embedding skips with no_text

### Requirement: Usable stage outcomes
The system SHALL expose readable stage outcomes in the web and Apple document details and shared workflow builder.

#### Scenario: Retry visibility
- **WHEN** a stage awaits another attempt
- **THEN** both clients display Waiting to retry with the attempt and safe reason

#### Scenario: Selected failure trigger
- **WHEN** a configured stage reaches terminal failure
- **THEN** the shared workflow engine evaluates the selected stage conditions once per source event and rechecks action authorization

### Requirement: Read-only processing detail
GET `/api/documents/{id}` SHALL include read-only current-revision stage summaries when available and preserve existing authorization.

#### Scenario: Authorized detail read
- **WHEN** an authorized reader requests a live document with processing runs
- **THEN** the API returns 200 with stage, run and revision identity, attempt, outcome, safe reason, and retry time

#### Scenario: Unauthenticated detail read
- **WHEN** an unauthenticated caller requests document processing detail
- **THEN** the API returns 401 without processing data

#### Scenario: Invisible document
- **WHEN** a caller requests a document outside their accessible vaults
- **THEN** the API returns 404 without processing data
