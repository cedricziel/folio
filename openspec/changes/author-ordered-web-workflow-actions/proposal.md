## Why

Workflow runs execute actions in order, but the web editor exposes only the first action of each kind. Users need to edit repeated actions and change their order while keeping saved webhook credentials with the intended destination.

## What changes

The web workflow form adds, moves, and removes actions. It checks which actions the selected trigger supports. Changes to a saved workflow use its definition revision to resolve masked webhook values and detect concurrent updates.

## Impact

The folio-api HTML routes and templates handle ordered rows. The HTML edits use the same transactional revision check as the guarded definition API. Existing workflow records require no migration.
