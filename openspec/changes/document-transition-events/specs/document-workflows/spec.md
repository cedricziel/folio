## ADDED Requirements

### Requirement: Document transition triggers

Workflows SHALL select specific title, tag, correspondent, document type, move, trash, restore, purge-requested, and purge-completed events in both the web and Apple builders.

#### Scenario: Classification replacement
- **WHEN** a document's tags change from A and B to B and C
- **THEN** one tag transition contains before A/B, after B/C, added C, and removed A
- **AND** conditions can match added C without matching retained B

#### Scenario: No-op and rollback
- **WHEN** an assignment is unchanged, a tag set is reordered, or the mutation rolls back
- **THEN** no transition event or workflow run is created

#### Scenario: Generic compatibility
- **WHEN** a title change produces a specific transition and a compatible generic update
- **THEN** a workflow configured for either trigger executes at most once for that transition

### Requirement: Durable authorized occurrence handling

Transition events SHALL retain immutable occurrence context and retry-stable identities. Actions SHALL re-check current authorization and current subject availability.

#### Scenario: Vault move
- **WHEN** a document moves between vaults
- **THEN** each vault receives only its own occurrence snapshot
- **AND** a source-vault workflow cannot mutate the destination document

#### Scenario: Purge retry
- **WHEN** a purge request or outbox delivery retries
- **THEN** the request and completion remain distinct facts
- **AND** each workflow consumes each occurrence at most once

#### Scenario: Delayed delivery
- **WHEN** an event is consumed after its document was trashed, moved, or purged
- **THEN** conditions evaluate the retained occurrence snapshot
- **AND** unsupported actions produce readable errors in both clients
