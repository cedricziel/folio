## Why

Vault owners need workflows that fire on a UTC clock, including when the application restarts after an occurrence comes due.

## What changes

Add one-time and recurring clock triggers with webhook actions. Store each clock's next instant and commit it with the corresponding runs and jobs. Use the existing definition revision to stop superseded runs.

## Impact

The workflow API and web editor accept an optional cadence for the scheduled trigger. SQLite and PostgreSQL use the matching migration `20261123000000_workflow_clocks.sql`. Clock triggers have no document or event subject. Document date offsets and the Apple schedule editor follow this slice.
