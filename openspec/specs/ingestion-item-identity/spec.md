# ingestion-item-identity Specification

## Purpose
TBD - created by archiving change ingestion-item-identity. Update Purpose after archive.
## Requirements
### Requirement: Stable source observations
Ingestion adapters SHALL pass opaque item identity independently of content deduplication while preserving existing import, veto, and external cleanup behavior.

#### Scenario: A staged folder item resumes after restart
- **WHEN** a source resumes a staged file after a busy or interrupted import
- **THEN** the item retains its observation identity
- **AND** a same-path replacement has a different identity even when its bytes are equal

#### Scenario: A mailbox is polled again
- **WHEN** the same UID in the same mailbox generation is retried
- **THEN** each attachment retains its identity
- **AND** a UIDVALIDITY change creates a distinct identity

#### Scenario: Allowed attachment types change
- **WHEN** a configuration change includes an earlier attachment in the same message
- **THEN** the later attachment retains its identity

#### Scenario: An existing sink receives an observed item
- **WHEN** a sink has not implemented observation-aware processing
- **THEN** it receives the item through its existing ingest implementation without changing the result

