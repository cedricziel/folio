## ADDED Requirements

### Requirement: Vault-owned ingestion outcome workflows

Ingestion outcome and source health triggers SHALL accept webhook actions only. The workflow engine SHALL match authorized owners-audience occurrences using captured source and outcome data. Before each action it SHALL verify the vault is active, ownership is current, the source remains enabled in that vault, and the workflow still authorizes the queued actions. Payloads SHALL omit filenames, paths, credentials, and provider errors.

#### Scenario: Distinct attempts and replay

- **WHEN** the same ingestion item fails on two distinct committed attempts
- **THEN** each attempt can queue a run
- **AND** replaying either attempt does not queue another run

#### Scenario: Source trashed after enqueue

- **WHEN** a managed source moves to Trash before delivery
- **THEN** Folio fails its queued outcome run without attempting the webhook
