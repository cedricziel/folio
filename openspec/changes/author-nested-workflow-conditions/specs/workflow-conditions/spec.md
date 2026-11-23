## ADDED Requirements

### Requirement: Web authoring of nested workflow conditions

The web workflow builder SHALL let users add and remove all, any, and none groups and edit filters or subgroups within their branches.

#### Scenario: Save and reopen nested conditions

- **WHEN** a user saves a workflow with nested groups through the web editor
- **THEN** reopening it shows the saved groups and branch filters
- **AND** existing predicates retain their typed values

#### Scenario: Keep or remove an empty alternative

- **WHEN** a user saves an explicitly empty any group
- **THEN** the saved group never matches
- **AND** removing the group removes that constraint

#### Scenario: Submit a legacy form

- **WHEN** a submitted form omits the nested condition editor
- **THEN** an edit retains the saved groups

#### Scenario: Invalid nested conditions

- **WHEN** the web form contains an invalid nested predicate or exceeds the condition limits
- **THEN** the editor shows a validation error and leaves the saved definition intact
