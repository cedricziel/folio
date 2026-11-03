## Why

A committed mutation can publish several event projections. Source-row deduplication alone can execute one workflow more than once when it selects multiple projections.

## What changes

Persist the captured mutation identity on new runs and enforce one run per workflow and mutation in the same transaction that queues execution. Preserve legacy source-row deduplication. Resolve registered domain and legacy trigger candidates together without duplicates.

## Impact

Touches folio-core run identity and trigger registry, folio-infra repositories and runner. Both databases add `20261102165000_workflow_mutation_dedup.sql`. Existing rows retain a null mutation ID and their source-row constraint; history is not rewritten. No HTTP route or global configuration changes. Generic dispatch and multiple-trigger editors remain in expand-workflow-builder.
