## Why

Entry events carry body-free occurrence identity but no document metadata snapshot. Workflows need typed event filters that do not fetch current metadata or confuse system attribution with missing event context.

## What changes

Add optional subject-ID and system/user actor conditions. Evaluate event and document predicates within the same recursive condition branch. Reject unknown event-filter fields and pre-import definitions that require a committed event. Existing document workflow dispatch uses the captured envelope for these filters.

## Impact

Touches core condition models and matching, workflow dispatch, OpenAPI and generated Swift types. No producer, migration, or authorization grant is added. Six entry-trigger dispatch and shared editor controls remain required follow-up work in expand-workflow-builder.
