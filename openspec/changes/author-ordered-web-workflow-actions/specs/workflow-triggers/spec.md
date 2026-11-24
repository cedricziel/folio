## ADDED Requirements

### Requirement: Ordered actions in the web workflow builder

The web builder SHALL let users add, move, and remove supported actions. It SHALL save the actions in the displayed order and show that order when the workflow is reopened.

#### Scenario: Reorder repeated actions

- **WHEN** a user adds multiple actions of the same kind and moves or removes a row
- **THEN** the saved definition contains the remaining rows in the order shown in the builder

#### Scenario: Reorder masked webhooks

- **WHEN** a user moves saved webhook rows while retaining their masked values
- **THEN** the server restores each row's credentials from its original location in the loaded definition revision
- **AND** a destination change prevents reuse of that row's masked values

#### Scenario: Save from an outdated form

- **WHEN** a user submits an ordered action list based on an outdated revision
- **THEN** the web route responds with HTTP 409 and shows the current saved workflow

#### Scenario: Update races with a definition edit

- **WHEN** the workflow changes after the web route resolves masked values and before it saves the update
- **THEN** the web route responds with HTTP 409 and leaves the newer definition intact
