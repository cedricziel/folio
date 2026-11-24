## ADDED Requirements

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
