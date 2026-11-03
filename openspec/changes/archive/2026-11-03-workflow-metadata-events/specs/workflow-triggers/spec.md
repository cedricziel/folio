## ADDED Requirements

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
