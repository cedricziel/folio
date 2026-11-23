## Why

The workflow engine evaluates nested condition groups, but the web editor can only change the outer filters. An empty alternative group has a different meaning from a group that is absent.

## What changes

Let workflow authors add and remove all, any, and none groups and edit each branch using the applicable condition fields. Preserve existing typed predicates and empty alternative groups when a workflow is reopened.

## Impact

Changes the folio-api HTML workflow form and uses the existing folio-core condition model and validation. The JSON API, generated Swift schema, and database layout stay compatible.
