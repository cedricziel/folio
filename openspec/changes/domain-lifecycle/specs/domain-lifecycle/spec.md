## ADDED Requirements

### Requirement: Server-owned lifecycle metadata

Mutable domain entities SHALL expose creation, update, and deletion timestamps and nullable creation/update actors. Vault-owned objects SHALL expose their vault ID. Clients SHALL NOT control attribution.

#### Scenario: Authenticated mutation
- **WHEN** an authenticated user changes an object in an authorized vault
- **THEN** the server records that user and its own update time without changing creation metadata

#### Scenario: Legacy or system mutation
- **WHEN** a migration cannot establish an actor or a background job changes an object
- **THEN** the relevant actor is null and no historical user is invented

### Requirement: Retained deletion

Deleted domain objects SHALL be omitted from active selectors while preserving existing references. Live names SHALL be unique within a vault. Restoration SHALL reject name conflicts.

#### Scenario: Reused classification name
- **WHEN** a deleted classification's name has been reused
- **THEN** restoration reports a conflict and retains both identities unchanged

### Requirement: Vault isolation

Mutations SHALL reject references outside the authorized vault. Moves SHALL require access to both vaults and preserve immutable historical records.

#### Scenario: Foreign classification
- **WHEN** a document update supplies a classification from another vault
- **THEN** the update fails without changing the document

### Requirement: Usable lifecycle controls

The server-rendered web and Apple app SHALL display lifecycle state and offer authorized deletion and restoration controls.

#### Scenario: Restore failure
- **WHEN** restoration conflicts with a live name
- **THEN** the interface explains the conflict and keeps the deleted object visible

### Requirement: Transactional domain events

Committed domain transitions SHALL publish versioned, vault-scoped events through the existing durable outbox. Events SHALL retain stable identity, subject, occurrence time, server-derived actor, and causation information across retries. Non-document events SHALL NOT imply a document action target.

#### Scenario: Mutation rollback or no-op
- **WHEN** a domain mutation rolls back or leaves persisted state unchanged
- **THEN** no transition event is published

#### Scenario: Retried delivery
- **WHEN** a dispatcher redelivers a committed event
- **THEN** consumers receive the same event identity and do not duplicate a workflow run

#### Scenario: Private subject
- **WHEN** an event describes user-private data
- **THEN** workflow consumers without subject access cannot observe its private payload
