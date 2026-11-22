## Why

Document notes and comments publish committed create, update, and delete occurrences. Vault owners need to select those transitions in workflows.

## What changes

Queue workflows from the six existing producer events. Match subject and actor conditions against captured event data. Deliver entry identity without its text or current document metadata. Recheck the active parent and vault before dispatch and execution. Suppress workflow- and import-origin occurrences.

## Impact

Changes folio-core trigger selection and matching, folio-infra execution guards, and the folio-api web editor and webhook schema. Existing workflows and document webhook payloads keep their wire values. Update OpenAPI and verify its generated Swift types. This change needs no migrations.
