## Why

Ingestion rules can veto imports synchronously, but durable workflows cannot observe item outcomes or source failure and recovery. Repeated polls must not create repeated outcome events.

## What changes

- Persist imported, skipped, duplicate, and failed item outcomes with stable item and attempt identities.
- Publish outcome and source failure/recovery events through the foundation transactional outbox.
- Preserve synchronous `ingest.received` veto and existing external email cleanup behavior.
- Show recent outcomes, retry state, and sanitized failure reasons in source views on web and Apple.
- Register readable triggers and bounded conditions with the workflow owner's shared builder.

## Impact

Changes folio-core ingestion models/ports/services, folio-infra SQLite/PostgreSQL adapters and folder/email sources, folio-api JSON/HTML source routes, OpenAPI, generated Swift client, and native source views. New migration `20261102171000_ingestion_outcomes.sql` in both backends adds immutable outcomes and operational item/attempt identity. Existing ledger rows remain valid and emit no historical events. Source status gains explicit vault context for configured sources resolved by the scheduler. No new setting.

Foundation envelope/publication and workflow registry are dependencies. Integrate only after their contracts land. Source CRUD/lifecycle publication stays with foundation.
