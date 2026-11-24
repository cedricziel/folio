## Why

Processing stage transactions publish typed outcomes that currently do not run workflows. Their captured document data omits source information.

## What changes

Use the merged catalog and producer payloads to dispatch processing outcomes. Match typed stage fields from the event and keep absent source information absent. Retain compatibility with legacy document projections, current parent checks, and origin suppression.

## Impact

Changes workflow matching and the webhook schema in folio-core, the infrastructure snapshot adapters and tests, and generated OpenAPI. Existing document webhook payloads continue to include their captured source when available. No migration or configuration change.
