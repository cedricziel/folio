## Why

Folio publishes ingestion item outcomes and source health transitions, but the workflow engine does not yet deliver them.

## What changes

Enable the six registered ingestion triggers. Use the captured event data for typed filters and webhook delivery. Recheck that the source, owning vault, and workflow remain authorized before running a webhook. Keep synchronous import rules separate from captured outcome workflows.

## Impact

Changes folio-core dispatch and conditions, folio-infra run execution, folio-app wiring, and folio-api workflow editing and OpenAPI. Existing document workflows retain their wire behavior. No migration or new configuration.
