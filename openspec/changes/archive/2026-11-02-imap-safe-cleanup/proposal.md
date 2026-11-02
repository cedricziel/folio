## Why

IMAP Move and Delete run mailbox-wide `EXPUNGE`, which can permanently remove unrelated messages that another mail client marked deleted. Issue #119 requires cleanup to affect only the imported UID.

## What Changes

- Prefer `UID MOVE` for Move when the server advertises MOVE.
- Require UIDPLUS for Delete and for the COPY/delete fallback used by Move, then use `UID EXPUNGE` for the imported UID.
- Leave messages unchanged when the server lacks the required capabilities, and log the cleanup failure.
- Document capability requirements and the temporary Mark Read mitigation for older versions.

## Capabilities

### New Capabilities

- `email-ingestion`: Cleanup removes only successfully imported messages and preserves unrelated mail.

## Impact

Touches the folio-infra IMAP adapter, isolated integration tests, and ingestion documentation. No folio-core ports or models, folio-api routes, settings, database migrations, JSON API, OpenAPI, or Swift client changes.
