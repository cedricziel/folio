## Why

Queued runs capture actions, but a definition edit can leave those actions eligible for execution. Scheduled occurrences will need the same revision check when they create a run.

## What changes

Persist a workflow definition revision and capture it with each matched run. Reject stale matches when inserting runs and jobs. Cancel older pending runs when a definition changes. Keep the original inputs and record the cancellation in run history.

## Impact

Adds the SQLite and PostgreSQL migration `20261102165100_workflow_definition_revision.sql`. Existing definitions keep their configuration and existing history keeps its inputs. No API fields change.
