# workflow-triggers Specification

## Purpose
Registers the events a workflow can react to and states which actions each trigger allows. Workflow definitions are validated against each trigger's allowed actions, and document metadata events are dispatched from the transactional outbox. Workflow authors and the workflow engine rely on it; compatibility here never grants permission.
## Requirements
### Requirement: Explicit trigger action compatibility
A registered trigger SHALL declare its subject, target compatibility, label, and typed condition fields. Registry validation SHALL reject unknown or duplicate trigger selections and actions incompatible with any selected trigger. Compatibility SHALL NOT grant authorization.

#### Scenario: Non-document event
- **WHEN** a selected event has no guaranteed mutable document target
- **THEN** document actions are rejected even if another selected trigger supports them

#### Scenario: Import candidate
- **WHEN** a workflow selects a pre-import candidate trigger
- **THEN** only skip-import is compatible

#### Scenario: Duplicate producer registration
- **WHEN** a producer registers an event name already in the catalog
- **THEN** registration fails without replacing the existing definition

#### Scenario: Invalid field catalog
- **WHEN** a producer registers duplicate condition field keys or an event name that does not match its subject
- **THEN** registration fails

#### Scenario: Unregistered event
- **WHEN** a workflow selects an event absent from the registry
- **THEN** validation rejects the selection

### Requirement: Precise document metadata dispatch
Workflows SHALL support document.title_changed, document.tags_changed, document.correspondent_changed, and document.document_type_changed from the existing transactional outbox. Matching and delivery SHALL use occurrence metadata without loading current metadata. Current document location SHALL still be checked before queuing and each action. Non-vault audiences SHALL remain excluded.

#### Scenario: Delayed producer event
- **WHEN** a document changes again before its earlier metadata transition is dispatched
- **THEN** conditions and delivery use the earlier producer snapshot

#### Scenario: Durable lineage
- **WHEN** a queued workflow action runs after its source outbox row is removed
- **THEN** emitted mutations retain source correlation and source-event causation under a null system actor

#### Scenario: Duplicate delivery
- **WHEN** the same producer outbox row is handled twice
- **THEN** only one workflow run is queued

### Requirement: Document discussion workflow occurrences

Workflows SHALL support vault-visible create, update, and delete occurrences for document notes and comments. Matching SHALL use the captured subject and actor. Delivery SHALL omit entry text and SHALL NOT synthesize a document snapshot. Dispatch and execution SHALL require an active parent document in the originating vault. Replayed occurrences SHALL NOT create another run for the same workflow. Workflows SHALL belong to the vault. Revoking the creator’s access SHALL remove their access to the workflow and its runs while leaving the active vault configuration eligible for execution.

#### Scenario: An edited comment

- **WHEN** a committed comment update meets an enabled workflow's event filters
- **THEN** Folio queues the run with the captured entry, parent, actor, and transition identities

#### Scenario: Parent moved before delivery

- **WHEN** a parent document moves out of the originating vault before dispatch or execution
- **THEN** the occurrence cannot execute in either vault

#### Scenario: Creator loses vault access

- **WHEN** the user who created an entry workflow loses access to the vault
- **THEN** that user can no longer read or edit the workflow or view its runs
- **AND** matching occurrences can still run while both the workflow and its parent document are active in the originating vault

### Requirement: Revision-checked native trigger edits

When an Apple workflow trigger is changed, the API SHALL require the revision returned with the workflow and SHALL reject stale revisions with HTTP 409. The edit SHALL retain the workflow's other settings. After a conflict, the editor SHALL allow reloading the current configuration before retrying.

#### Scenario: Another client turns the workflow off

- **WHEN** a vault owner saves a trigger after another client has turned the workflow off
- **THEN** the stale edit is rejected and the editor offers to reload the workflow
- **AND** saving the trigger after reloading leaves the workflow off

### Requirement: Durable UTC clock triggers

Vault owners SHALL be able to configure one-time and recurring UTC clocks for webhook workflows. Folio SHALL persist the next occurrence and atomically commit each advance with its immutable run and job. A clock occurrence SHALL be unique per workflow and UTC instant. Edits SHALL preserve an unchanged clock's cursor and stop runs from the older revision. Disabled clocks SHALL not enqueue runs. Re-enabling SHALL skip elapsed disabled occurrences.

#### Scenario: Two schedulers race at the same instant

- **WHEN** two scheduler instances claim the same due clock
- **THEN** one run and one job persist for that instant

#### Scenario: Restart after downtime

- **WHEN** a recurring clock has more than sixteen missed occurrences
- **THEN** Folio enqueues the latest sixteen and advances to the next aligned UTC instant

#### Scenario: Run creation rolls back

- **WHEN** creating the execution job fails
- **THEN** the run and processed cursor are rolled back together

#### Scenario: Enable a disabled workflow while editing its cadence

- **WHEN** a vault owner enables a disabled clock and sets a new cadence in the same update
- **THEN** the next occurrence is the first instant of the new cadence after the edit

#### Scenario: Convert a document workflow using explicit removals

- **WHEN** a vault owner selects a clock trigger and explicitly removes saved filters and actions that are incompatible with clocks in the same edit
- **THEN** Folio saves the clock and retains the workflow's webhooks in order

#### Scenario: Recurring clock cannot begin during a leap second

- **WHEN** an owner enters a leap-second instant as the start of a recurring clock
- **THEN** Folio rejects the definition before it can acquire a schedule cursor

#### Scenario: An event selection includes a schedule

- **WHEN** a workflow selects a clock or document date offset alongside another trigger
- **THEN** Folio rejects the configuration

### Requirement: Document date schedules

Vault owners SHALL be able to configure a signed offset in whole calendar days and a UTC time. At each occurrence, Folio SHALL select active documents in the workflow's vault whose document date matches the offset. The workflow SHALL apply its document conditions before queueing its ordered actions. Folio SHALL persist the document-page cursor in the same transaction as the queued runs and jobs.

#### Scenario: Restart while claiming a date

- **WHEN** Folio restarts with a saved document page cursor, even after the catch-up window has moved past that date
- **THEN** it finishes the saved occurrence and queues each remaining match once before applying the catch-up limit to later dates

#### Scenario: Offset schedule has an occurrence filter

- **WHEN** a vault owner saves a document date schedule with an event subject or actor condition
- **THEN** Folio rejects the definition and keeps any previous revision

