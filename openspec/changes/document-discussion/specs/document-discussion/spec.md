## ADDED Requirements

### Requirement: Document notes and comments remain distinct

The system SHALL provide editable notes and chronological comments attached to a logical document across file versions.

#### Scenario: Create an attributed entry
- **WHEN** a contributor submits valid text and kind to POST `/api/documents/{id}/entries`
- **THEN** the server returns 201 with the persisted entry and server-derived author and lifecycle timestamps
- **AND** the entry appears only in its selected Notes or Comments section

#### Scenario: Reject invalid input
- **WHEN** POST `/api/documents/{id}/entries` or PATCH `/api/documents/{id}/entries/{entry_id}` receives blank or oversized text
- **THEN** the server returns 400 without changing persisted content

### Requirement: Discussion access follows document and author permissions

The system SHALL enforce vault membership, live document visibility, authorship, and moderation permissions for every request.

#### Scenario: Protect another author's text
- **WHEN** a member attempts PATCH `/api/documents/{id}/entries/{entry_id}` for another author's entry
- **THEN** the server returns 403 without changing its body or audit fields

#### Scenario: Delete an entry
- **WHEN** an authorized author or editor sends DELETE `/api/documents/{id}/entries/{entry_id}`
- **THEN** the server returns 204 and records soft deletion and the acting user
- **AND** ordinary lists omit the deleted entry

#### Scenario: Reject a cross-vault entry identifier
- **WHEN** a client uses an entry ID belonging to another vault or document on PATCH or DELETE `/api/documents/{id}/entries/{entry_id}`
- **THEN** the server returns 404 without exposing or changing the entry

#### Scenario: Viewer cannot contribute
- **WHEN** a viewer sends POST `/api/documents/{id}/entries`
- **THEN** the server returns 403

### Requirement: Lists are bounded and scoped

The system SHALL paginate entries by kind and stable creation order, with authorization on every page.

#### Scenario: Continue chronological comments
- **WHEN** an authorized reader sends GET `/api/documents/{id}/entries` with kind comment and a valid continuation
- **THEN** the server returns 200 with a bounded chronological page and an optional next continuation

#### Scenario: Reject a reused continuation
- **WHEN** GET `/api/documents/{id}/entries` receives a continuation from another document, vault, or kind
- **THEN** the server returns 400

### Requirement: Web and Apple users can manage discussion

Both clients SHALL provide accessible read, add, edit, and delete flows with author and timestamp display, pagination, empty states, and recoverable errors.

#### Scenario: Retain a failed draft
- **WHEN** saving a note or comment fails
- **THEN** the editor retains its text and offers retry or cancel

#### Scenario: Read without write permission
- **WHEN** a viewer opens document discussion
- **THEN** entries and pagination remain available while mutation controls are absent

#### Scenario: Retain entries across file versions and trash
- **WHEN** a document receives a new file version or is trashed and restored
- **THEN** its surviving notes and comments remain attached to the same logical document

### Requirement: Entry transitions can trigger workflows safely

The system SHALL publish versioned document-targeted note/comment events through the existing transactional outbox and expose them in web and Apple workflow editors.

#### Scenario: Commit an entry mutation
- **WHEN** a note or comment is created, changed, or soft-deleted
- **THEN** its transaction persists exactly one event for that transition with entry and document identities, vault scope, time, and server-derived actor
- **AND** the payload excludes entry body text

#### Scenario: Retry or roll back a mutation
- **WHEN** a mutation rolls back, repeats a deletion, submits unchanged text, or retries an already committed idempotency identity
- **THEN** no duplicate entry or transition event is persisted

#### Scenario: Workflow owner loses access
- **WHEN** an entry event is delivered after its document moves out of the workflow owner's authorized vault or their membership is revoked
- **THEN** the workflow does not execute document actions or expose entry data to that owner

#### Scenario: Select entry triggers
- **WHEN** an authorized user edits a workflow in either client
- **THEN** note and comment creation, update, and deletion are selectable with readable labels and supported condition fields
