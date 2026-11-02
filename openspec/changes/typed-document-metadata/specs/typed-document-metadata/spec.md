## ADDED Requirements

### Requirement: Vault managers can manage typed fields

The system SHALL provide vault-owned fields for text, long text, boolean, integer, decimal, date, money, select, URL, and document links with lifecycle timestamps and server-derived audit actors.

#### Scenario: Create and list a field
- **WHEN** a vault manager POSTs a valid definition to `/api/vaults/{vault_id}/metadata-fields`
- **THEN** the API returns 201 with a stable ID and GET on the collection returns 200 including that definition

#### Scenario: Reject inaccessible management
- **WHEN** a caller lacks authentication, management permission, or access to the requested vault
- **THEN** POST returns 401, 403, or 404 respectively without disclosing another vault's definitions

#### Scenario: Retire and restore a field
- **WHEN** a manager DELETEs `/api/vaults/{vault_id}/metadata-fields/{field_id}` then POSTs its `/restore` endpoint
- **THEN** retirement returns 204 and preserves values, and restoration returns 200 or 409 if its name was reused

#### Scenario: Prevent reinterpretation
- **WHEN** PATCH `/api/vaults/{vault_id}/metadata-fields/{field_id}` changes the kind of a field with retained values
- **THEN** the API returns 409 and preserves the definition and values

### Requirement: Values are typed, exact, and atomic

The system SHALL validate typed values and persist them without binary floating-point conversion. Null SHALL clear an optional value and omitted fields SHALL remain unchanged.

#### Scenario: Save exact values
- **WHEN** an editor PATCHes `/api/vaults/{vault_id}/documents/{document_id}/metadata` with valid typed values and the current revision
- **THEN** the API returns 200, stores the entire patch atomically, and GET returns 200 with exact canonical decimal strings and explicit money currency

#### Scenario: Reject invalid values
- **WHEN** a patch contains a mismatched kind, impossible date, excessive precision, invalid URL, or null for a required field
- **THEN** PATCH returns 400 and no value changes

#### Scenario: Reject uniqueness and stale edits
- **WHEN** a patch duplicates a unique canonical value or uses a stale metadata revision
- **THEN** PATCH returns 409 and no value changes

### Requirement: Select options retain stable identity

The system SHALL preserve option IDs across renames and retirement, retain existing assignments, and reject new assignments of retired options.

#### Scenario: Rename an option
- **WHEN** a manager PATCHes `/api/vaults/{vault_id}/metadata-fields/{field_id}/options/{option_id}` with a new label
- **THEN** the API returns 200 and existing document values display the new label with the same option ID

#### Scenario: Reject an unrelated option
- **WHEN** a metadata PATCH assigns an option from another field or vault
- **THEN** the API returns 404 without changing any values

### Requirement: Document links preserve vault and logical identity

The system SHALL attach links to stable logical document IDs and enforce access without leaking information about inaccessible targets.

#### Scenario: Reject hidden targets
- **WHEN** a metadata PATCH links to a missing, inaccessible, or cross-vault target
- **THEN** the API returns the same 404 response for each case

#### Scenario: Retain links across revisions and trash
- **WHEN** a linked document receives a file revision or enters trash
- **THEN** GET metadata returns 200 with the same logical target ID and an authorized trash indicator where applicable

#### Scenario: Purge or move a linked document
- **WHEN** a target is permanently purged
- **THEN** incoming values are cleared transactionally and no dangling target remains
- **WHEN** a move would leave retained metadata or links crossing vault boundaries
- **THEN** the move returns 409 without changing vault ownership

### Requirement: Users can manage and edit fields in both clients

The web application and native Apple app SHALL provide field management, typed document editing, display, and authorized link navigation with the shared design system.

#### Scenario: Edit a document field
- **WHEN** an editor opens the Custom fields section and saves a typed value
- **THEN** both clients display the persisted value, preserve failed drafts, and distinguish false from missing

#### Scenario: Reader navigation
- **WHEN** a reader opens a document with custom fields
- **THEN** both clients display values and permit authorized document-link navigation without showing mutation controls

### Requirement: Exact metadata search remains bounded and scoped

The system SHALL support bounded cursor pagination for an exact typed value in one authorized vault.

#### Scenario: Search by typed value
- **WHEN** GET `/api/vaults/{vault_id}/metadata-search` supplies a field ID, typed exact value, and valid limit
- **THEN** the API returns 200 with only matching active documents in that vault and a query-bound continuation cursor

#### Scenario: Reject an unavailable field
- **WHEN** metadata search references a missing or retired field
- **THEN** the API returns 404 instead of broadening the query
