# document-workflows Specification

## Purpose
Define how Folio builds, schedules, and executes vault-scoped document workflows.
## Requirements
### Requirement: Compatible workflow definitions
The system SHALL accept multiple well-defined triggers with ordered actions and bounded all, any, and none condition groups while preserving legacy definitions.

#### Scenario: Legacy API definition
- **WHEN** a writer submits a valid legacy definition to POST /api/workflows
- **THEN** the API returns 201 and preserves its trigger, conditions, and ordered actions

#### Scenario: Incompatible action
- **WHEN** a writer submits skip-import with a document trigger to PUT /api/workflows/{id}
- **THEN** the API returns 400 without changing the stored definition

### Requirement: Durable authorized execution
The system SHALL persist immutable run inputs, prevent recursive workflow updates, and cancel stale or unauthorized work before another attempt.

#### Scenario: Retry after partial completion
- **WHEN** a later action fails after an earlier action succeeds
- **THEN** the retry resumes from durable progress with stable delivery identity and idempotent document mutations

#### Scenario: Deleted definition
- **WHEN** a queued run belongs to a deleted definition
- **THEN** no further action executes and history records the cancellation

### Requirement: Restart-safe scheduled triggers
The system SHALL persist occurrence claims for scheduled, recurring, and document-date-offset triggers.

#### Scenario: Competing schedulers after restart
- **WHEN** two schedulers claim the same due occurrence
- **THEN** only one durable run is queued and the next occurrence survives restart

### Requirement: Equivalent workflow builders
The web and Apple applications SHALL support trigger selection, typed conditions, ordered actions, enable controls, and run history with vault permissions.

#### Scenario: Edit an existing ordered definition
- **WHEN** a writer reorders actions and saves in either application
- **THEN** reopening displays the saved order and preserves unrelated definition fields

#### Scenario: Read-only vault member
- **WHEN** a viewer opens workflows
- **THEN** definitions and allowed run history are readable and mutation controls are unavailable

### Requirement: Integrated document automation
The system SHALL support typed metadata and document date conditions, metadata set/clear actions, reviewed state actions, and review/version triggers using their domain services.

#### Scenario: Workflow mutation emits an event
- **WHEN** a workflow changes metadata or reviewed state
- **THEN** the mutation carries server-derived attribution and does not recursively enqueue workflows

### Requirement: Shared domain event triggers
The system SHALL consume versioned committed domain events from the existing durable outbox and expose supported transitions as triggers in both builders.

#### Scenario: Non-document trigger with document action
- **WHEN** a writer submits a non-document trigger with a rename-document action to POST /api/workflows
- **THEN** the API returns 400 without creating the definition

#### Scenario: Restricted event
- **WHEN** an event concerns a private saved view or discussion that the workflow execution authority cannot access
- **THEN** no matching, run history, or webhook delivery exposes the event or its condition data

#### Scenario: Transactional workflow lifecycle
- **WHEN** a workflow definition mutation commits a state transition
- **THEN** its shared domain event is committed atomically with stable identity, occurrence time, vault and subject scope, nullable server-derived actor, and causation
- **AND** rollback or a no-op produces no event

#### Scenario: Domain event replay
- **WHEN** the dispatcher redelivers a committed domain event after restart
- **THEN** the workflow does not queue a duplicate run or recursively respond to its own causal changes

### Requirement: Note and comment workflow integration
The workflow catalog, dispatch, and both builders SHALL support `document_note.created`, `document_note.updated`, `document_note.deleted`, `document_comment.created`, `document_comment.updated`, and `document_comment.deleted`. Integration SHALL consume producer-owned envelopes without duplicate publication. The entry is the subject and `document_id` identifies its parent. Condition catalogs and delivery payloads SHALL NOT expose entry bodies. Document-target actions SHALL require current parent-document authorization in addition to event visibility.

#### Scenario: Actual producer envelope dispatch
- **WHEN** a note or comment producer commits one of the six transitions on either database
- **THEN** an integration regression consumes that actual envelope through workflow dispatch and verifies the selected trigger and body-free condition data

#### Scenario: Parent access revoked
- **WHEN** the execution authority loses parent-document access before an action runs
- **THEN** the document action does not execute

#### Scenario: Builder roundtrip
- **WHEN** a user selects any of the six entry transitions in the web or Apple builder
- **THEN** saving and reopening preserves the selection and exposes only supported condition fields and actions

#### Scenario: Entry replay and recursion
- **WHEN** the same committed entry envelope is replayed or an entry event originates from workflow execution
- **THEN** replay creates no duplicate run and workflow-origin recursion is suppressed

#### Scenario: Parent moved before delivery
- **WHEN** an entry event is dispatched after its parent leaves the event's vault
- **THEN** it does not trigger actions or disclose entry context across vaults

#### Scenario: Unsupported document conditions
- **WHEN** an entry trigger has only entry and envelope occurrence fields
- **THEN** document metadata conditions are unavailable and current metadata is not substituted for a historical snapshot

### Requirement: Envelope and compatibility trigger candidates
Dispatch SHALL resolve registered candidates from both a domain envelope and its legacy projection. A single workflow matching multiple candidates from one mutation SHALL create one durable run. Distinct workflows MAY each run. Legacy rows without envelopes SHALL retain source-event deduplication. Authorization SHALL apply to every candidate.

#### Scenario: Purge completion compatibility
- **WHEN** a document.purged envelope carries a legacy Deleted projection
- **THEN** both purged and document.deleted workflows can match, while a workflow selecting both creates only one run

#### Scenario: Distinct mutations in one causal chain
- **WHEN** two matching mutations share a correlation ID
- **THEN** each mutation remains eligible for its own run
