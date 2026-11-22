## ADDED Requirements

### Requirement: Keep open document lists consistent with the current view

When a document is created, removed, or changed by processing or metadata edits, the browser SHALL refresh an open document list using the current vault, destination, filters, sort, and page. The refresh SHALL update rows, the empty state, and vault counts without navigating away or clearing an open editor.

#### Scenario: A document starts or stops matching

- **GIVEN** an open filtered list or saved view
- **WHEN** a document is created or changed in its vault
- **THEN** the list shows that document only if it matches the current results
- **AND** the empty state is visible exactly when the current results are empty
- **AND** edits in an open form remain in place

#### Scenario: Processing changes Inbox membership

- **GIVEN** the Inbox shows a reviewed document that is still processing
- **WHEN** processing finishes successfully
- **THEN** the document leaves the Inbox
- **AND** the Inbox count updates while the All documents count still includes the document
