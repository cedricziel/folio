## ADDED Requirements

### Requirement: Document discussion workflow occurrences

Workflows SHALL support vault-visible create, update, and delete occurrences for document notes and comments. Matching SHALL use the captured subject and actor. Delivery SHALL omit entry text and SHALL NOT synthesize a document snapshot. Dispatch and execution SHALL require an active parent document in the originating vault. Replayed occurrences SHALL NOT create another run for the same workflow. Workflows SHALL belong to the vault. Revoking the creator’s access SHALL remove their access to the workflow and its runs while leaving the active vault configuration eligible for execution.

#### Scenario: An edited comment

- **WHEN** a committed comment update meets an enabled workflow's event filters
- **THEN** Folio queues the run with the captured entry, parent, actor, and transition identities

#### Scenario: Parent moved before delivery

- **WHEN** a parent document moves out of the originating vault before dispatch or execution
- **THEN** the occurrence cannot execute in either vault

#### Scenario: Creator loses vault access

- **WHEN** the user who created an entry workflow loses access to the vault
- **THEN** that user can no longer read or edit the workflow or view its runs
- **AND** matching occurrences can still run while both the workflow and its parent document are active in the originating vault

### Requirement: Revision-checked native trigger edits

When an Apple workflow trigger is changed, the API SHALL require the revision returned with the workflow and SHALL reject stale revisions with HTTP 409. The edit SHALL retain the workflow's other settings. After a conflict, the editor SHALL allow reloading the current configuration before retrying.

#### Scenario: Another client turns the workflow off

- **WHEN** a vault owner saves a trigger after another client has turned the workflow off
- **THEN** the stale edit is rejected and the editor offers to reload the workflow
- **AND** saving the trigger after reloading leaves the workflow off
