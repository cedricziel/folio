# workflow-triggers Specification

## Purpose
TBD - created by archiving change workflow-trigger-catalog. Update Purpose after archive.
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

