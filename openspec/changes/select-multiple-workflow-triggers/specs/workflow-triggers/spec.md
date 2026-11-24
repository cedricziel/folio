## ADDED Requirements

### Requirement: Multiple workflow event triggers

Workflows SHALL accept a nonempty ordered list of distinct triggers and require every selected trigger to support every configured action and event condition. Definitions SHALL continue to accept one legacy trigger. When a request supplies both fields, the legacy trigger SHALL equal the first selected trigger.

#### Scenario: Edit a multi-trigger workflow using an older client

- **WHEN** a client updates a workflow and supplies its unchanged primary trigger without a trigger list
- **THEN** Folio keeps the additional triggers in their saved order

#### Scenario: Two selected projections describe the same mutation

- **WHEN** a workflow selects both projections and their occurrence belongs to one mutation
- **THEN** Folio creates one run for that mutation and workflow

#### Scenario: A secondary event cannot run the actions

- **WHEN** any selected trigger disallows a configured action or event condition
- **THEN** Folio rejects the configuration
