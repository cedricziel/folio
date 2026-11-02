# Email ingestion specification

## Purpose

Keep IMAP post-import cleanup scoped to imported messages and preserve unrelated mailbox contents.

## Requirements

### Requirement: Cleanup preserves unrelated messages

After successfully importing a message, the system SHALL restrict Move and Delete cleanup to that message's UID. It SHALL NOT issue mailbox-wide EXPUNGE or CLOSE for cleanup.

#### Scenario: Delete with UIDPLUS

- **GIVEN** the server advertises UIDPLUS and an unrelated message is already marked deleted
- **WHEN** Folio imports a message with Delete selected
- **THEN** only the imported UID is expunged and the unrelated message retains its flags

#### Scenario: Move with MOVE

- **GIVEN** the server advertises MOVE and an unrelated message is already marked deleted
- **WHEN** Folio imports a message with Move selected
- **THEN** UID MOVE transfers only the imported message to the configured destination
- **AND** the unrelated message remains in the source mailbox with its flags unchanged

#### Scenario: Move with UIDPLUS but without MOVE

- **GIVEN** the server advertises UIDPLUS but not MOVE and an unrelated message is already marked deleted
- **WHEN** Folio imports a message with Move selected
- **THEN** Folio copies the imported UID and expunges only that UID from the source mailbox
- **AND** the unrelated message retains its flags

### Requirement: Unsupported cleanup leaves mail unchanged

The system SHALL leave source messages and destination mailboxes unchanged when the server lacks the capabilities required for safe cleanup. It SHALL log the cleanup failure and retain the import ledger entry.

#### Scenario: Delete without UIDPLUS

- **GIVEN** the server does not advertise UIDPLUS
- **WHEN** Folio imports a message with Delete selected
- **THEN** the message is imported but its source flags remain unchanged
- **AND** cleanup logs a failure

#### Scenario: Move without MOVE or UIDPLUS

- **GIVEN** the server advertises neither MOVE nor UIDPLUS
- **WHEN** Folio imports a message with Move selected
- **THEN** Folio leaves the source message unchanged and creates no destination copy
- **AND** cleanup logs a failure

#### Scenario: Scoped cleanup command rejected

- **GIVEN** the server advertises a safe cleanup capability but rejects the corresponding command
- **WHEN** Folio attempts cleanup
- **THEN** cleanup logs the error and does not fall back to a mailbox-wide expunge

