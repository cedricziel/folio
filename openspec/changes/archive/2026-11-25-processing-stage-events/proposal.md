## Why

Processing status writes and outbox appends use separate transactions. OCR and thumbnail events do not trigger workflows, transient failures look terminal, and successful processing can hide skipped stages.

## What changes

- Persist bounded processing stage transitions with run, attempt, and revision identity.
- Commit status, result references, queue outcome, and domain events atomically with lease and revision fencing.
- Register readable OCR, embedding, and thumbnail outcome triggers in the shared workflow builder.
- Show stage state, retry state, and sanitized errors in web and Apple document detail.

## Impact

Touches folio-core models and ports, folio-infra SQLite and PostgreSQL adapters and workers, folio-api JSON and HTML document detail, OpenAPI, and the generated Swift client and native views. Add new `processing_stage_runs` migrations in both database directories after foundation and revision migrations land. Existing status remains readable; historical outcomes are not fabricated. No new settings or event bus.
