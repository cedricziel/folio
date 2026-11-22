## ADDED Requirements

### Requirement: Revision checks for workflow execution

Folio SHALL associate new runs with the definition revision captured at matching. Folio SHALL check the captured revision when it enqueues the run and before it executes each action. A successful definition change SHALL cancel pending runs and preserve their captured inputs and completed actions. A no-op or rolled-back edit SHALL leave the revision and pending runs unchanged.

#### Scenario: A definition changes after matching

- **WHEN** a workflow matches an occurrence and is edited before the run is persisted
- **THEN** the stale match creates neither a run nor a job

#### Scenario: A workflow is disabled and re-enabled

- **WHEN** an owner disables a workflow and then re-enables it with its previous configuration
- **THEN** a match captured before disablement cannot create a run

#### Scenario: An edit supersedes a pending run

- **WHEN** an owner edits a definition with a pending run
- **THEN** the run becomes terminal with a reason in history
- **AND** the captured actions and event data are retained
