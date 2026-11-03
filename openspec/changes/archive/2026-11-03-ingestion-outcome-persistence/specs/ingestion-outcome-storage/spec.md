## ADDED Requirements

### Requirement: Terminal outcomes commit atomically
The repository SHALL commit one imported, skipped, duplicate, or failed outcome per attempt with its existing-outbox event and any content-ledger transition in one transaction.

#### Scenario: Publication fails
- **WHEN** inserting the terminal event fails
- **THEN** the outcome, item state, and content-ledger transition are rolled back

#### Scenario: Completion is replayed
- **WHEN** a committed attempt is completed again with the same result
- **THEN** its stored outcome and event identity are returned without another event

#### Scenario: A stale attempt completes
- **WHEN** a newer attempt has reclaimed the item or content reservation
- **THEN** the stale attempt cannot publish or change the ledger

### Requirement: Source authority is explicit
Managed claims and completion SHALL recheck source vault, kind, and enabled state. Configured sources SHALL be identified explicitly by trusted callers, never inferred from an absent managed row.

#### Scenario: A managed source is disabled
- **WHEN** a previously claimed managed source becomes disabled
- **THEN** its completion does not publish an outcome

### Requirement: Source health emits transitions
Source status, durable health state, and failure/recovery events SHALL commit atomically. Events SHALL use owner-only audiences and bounded identifiers and reason codes without private input.

#### Scenario: Failure repeats
- **WHEN** an unavailable source reports another error with different diagnostic text
- **THEN** no additional failure event is emitted until recovery

#### Scenario: Initial healthy poll
- **WHEN** a source with no previous health state reports success
- **THEN** no recovery event is emitted

### Requirement: Upgrades retain existing ingestion state
Both database migrations SHALL retain existing content-ledger rows without backfilling historical outcomes or events.

#### Scenario: An existing archive is upgraded
- **WHEN** migrations run over a populated legacy ledger
- **THEN** existing ledger state is retained and new outcome history is empty
