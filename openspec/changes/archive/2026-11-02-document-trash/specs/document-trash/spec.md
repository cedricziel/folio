## ADDED Requirements

### Requirement: Default deletion is recoverable
The system SHALL move documents to Trash by default and retain them until explicitly deleted forever.

#### Scenario: Delete a document
- **WHEN** DELETE `/api/documents/{id}` targets an accessible active document
- **THEN** it returns 204 and the document disappears from ordinary lists, search, downloads, and processing without removing its files or metadata
- **AND** repeating deletion while it remains trashed returns 204

#### Scenario: List and restore trash
- **WHEN** GET `/api/documents/trash` is requested
- **THEN** it returns 200 with only the selected vault's trashed documents, including purge-pending entries
- **WHEN** POST `/api/documents/{id}/restore` targets a recoverable trashed document
- **THEN** it returns 200 and restores normal visibility, metadata, and files

### Requirement: Permanent deletion is explicit and durable
The system SHALL persist an explicit purge request before deleting any stored files and SHALL retry incomplete cleanup.

#### Scenario: Request permanent deletion
- **WHEN** DELETE `/api/documents/{id}/purge` targets an accessible trashed document
- **THEN** it returns 202 after durable intent and work are recorded
- **AND** POST `/api/documents/{id}/restore` returns 409 after that intent

#### Scenario: Reject invalid or unauthorized actions
- **WHEN** restore or purge targets a document outside the selected vault or outside Trash
- **THEN** the JSON endpoint returns 404
- **WHEN** a user lacks document deletion permission
- **THEN** trash, restore, and purge return 403 without mutation

#### Scenario: Storage cleanup fails
- **WHEN** permanent storage deletion fails
- **THEN** the trash row and purge intent remain and retries continue without exposing a broken active document
- **AND** database removal occurs only after storage cleanup succeeds

#### Scenario: Processing and deletion overlap
- **WHEN** a document is trashed during processing
- **THEN** queued processing skips it and its existing files remain recoverable
- **AND** permanent deletion waits for active processing so late writes cannot recreate deleted files

### Requirement: Trash is available in the web interface
The system SHALL provide a Trash page with Restore and separately confirmed Delete forever actions.

#### Scenario: Pending deletion
- **WHEN** permanent deletion has been requested
- **THEN** Trash shows Deleting permanently and offers no restore or delete action for that row

#### Scenario: Live changes
- **WHEN** a document is trashed, restored, or permanently deleted
- **THEN** explicit outbox events update open document lists and Trash without crossing vault boundaries
