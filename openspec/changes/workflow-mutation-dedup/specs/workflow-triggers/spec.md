## ADDED Requirements

### Requirement: Mutation-scoped workflow execution
New runs with a captured domain identity SHALL be unique by workflow and mutation. Run persistence and job enqueue SHALL be atomic. Correlation identity SHALL NOT deduplicate distinct mutations. Legacy runs without domain identity SHALL retain source-event deduplication. Invalid captured identities SHALL fail before enqueue.

#### Scenario: Concurrent projections
- **WHEN** two deliveries from one mutation concurrently match the same workflow
- **THEN** exactly one run and one job are persisted

#### Scenario: Separate workflows and mutations
- **WHEN** separate workflows match one mutation or one workflow matches distinct mutations sharing a correlation
- **THEN** each workflow and mutation pair remains eligible

### Requirement: Combined trigger candidates
The registry SHALL resolve registered domain and legacy trigger names together, preserving domain-first order and removing duplicates. Candidate resolution SHALL NOT grant execution authority.

#### Scenario: Purge compatibility
- **WHEN** document.purged and its legacy document.deleted projection are registered
- **THEN** both are candidates for the same occurrence

#### Scenario: Legacy event
- **WHEN** an event has no domain envelope
- **THEN** its registered legacy trigger remains a candidate
