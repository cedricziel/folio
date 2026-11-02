## ADDED Requirements

### Requirement: Date-only document metadata
The system SHALL store an optional document date separately from immutable import created_at. JSON PATCH `/api/documents/{id}` SHALL accept document_date as YYYY-MM-DD or null, with omission preserving the existing value. The vault-scoped route SHALL enforce the same behavior.

#### Scenario: Clear and preserve dates
- **WHEN** an authorized editor sends PATCH `/api/documents/{id}` with document_date null
- **THEN** the response is 200, the document date is cleared, and created_at is unchanged
- **AND** a later PATCH omitting document_date preserves that state

#### Scenario: Invalid calendar value
- **WHEN** an editor sends PATCH `/api/documents/{id}` with document_date 2025-02-29 or a timestamp
- **THEN** the response is 400 and metadata is unchanged

### Requirement: Explicit review decisions
The system SHALL persist review independently of OCR and classification. PUT `/api/documents/{id}/review` SHALL require a boolean reviewed and derive actor and time on the server. Repeating an unchanged review request SHALL preserve attribution. Legacy Filed membership SHALL carry migration provenance and no fictional reviewer.

#### Scenario: Review a processing document
- **WHEN** an authorized editor sends PUT `/api/documents/{id}/review` with reviewed true while OCR is pending
- **THEN** the response is 200 and records the server-derived review decision
- **AND** OCR remains pending and later classification does not alter the review

#### Scenario: Unreview a document
- **WHEN** an authorized editor sends PUT `/api/documents/{id}/review` with reviewed false
- **THEN** the response is 200 and clears review time, actor, and provenance

#### Scenario: Preserve access policy
- **WHEN** a caller without vault access sends PUT `/api/documents/{id}/review`
- **THEN** the response is 404 without disclosing document state

### Requirement: Date and review browsing
GET `/api/vaults/{vault_id}/documents` and GET `/api/documents/browse` SHALL apply review and inclusive document-date filters before pagination and bind them to continuation cursors. Existing import-date filters SHALL retain their meaning.

#### Scenario: Filter without an AI provider
- **WHEN** an authorized caller requests GET `/api/vaults/{vault_id}/documents` with reviewed=false and a valid date range while AI is local or unconfigured
- **THEN** the response is 200 and contains only matching unreviewed documents with dates in range

#### Scenario: Reject a changed continuation
- **WHEN** a caller changes a review or document-date filter while reusing a cursor on GET `/api/documents/browse`
- **THEN** the response is 400

### Requirement: Web and Apple review controls
Both clients SHALL expose date editing and clearing, review and unreview actions, review/date filters, and independent processing state. Controls SHALL retain selection and show retryable failures without pretending the write succeeded.

#### Scenario: Complete an Inbox review
- **WHEN** a user reviews an Inbox document in either client
- **THEN** the saved review state is visible, the Inbox query refreshes, and processing failures remain discoverable

### Requirement: Committed workflow events
The system SHALL publish domain-specific transitions through the existing durable outbox using the foundation's versioned, actor-attributed, vault-scoped envelope. Event identity SHALL survive delivery retries. Both workflow editors SHALL expose understandable trigger labels and supported condition fields. Mutations SHALL emit no event on rollback, unchanged state, or idempotent retry. Workflow-origin transitions SHALL retain recursion protection.

#### Scenario: Commit and retry
- **WHEN** a document transition commits and its request or delivery is retried
- **THEN** one event per applicable transition remains in the outbox with its original identity, occurrence time, actor, and immutable condition data

#### Scenario: Moved or deleted subject
- **WHEN** an event is consumed after the logical document moves or is deleted
- **THEN** workflow actions recheck subject access and cannot use the snapshot to access the old vault or resurrect deleted content
