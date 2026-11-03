## Why

The merged document metadata producers publish precise transitions, but workflows discard their domain-only outbox rows.

## What changes

Support title, tag, correspondent, and document-type transitions as workflow triggers. Match their immutable occurrence snapshots, preserve source event identity in queued payloads, and restore trusted lineage under the system actor during actions. Keep non-vault audiences excluded until explicit execution authority is implemented.

## Impact

Touches workflow models, engine, runner, OpenAPI, and workflow integration tests. Existing web trigger selection uses the shared trigger list. No new producer, migration, or publication path is added. Multi-trigger deduplication and the native builder remain tracked in expand-workflow-builder.
