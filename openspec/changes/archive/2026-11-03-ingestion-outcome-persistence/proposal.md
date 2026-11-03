## Why

Ingestion needs durable terminal outcomes and source health transitions before producers and user interfaces can use them. Repeated polling must not create repeated events.

## What Changes

- Add explicit vault/source/item/attempt persistence and bounded outcome reads on SQLite and PostgreSQL.
- Commit outcome, fenced content-ledger transition, and the existing transactional outbox event together.
- Commit source status, failure state, and failure/recovery events together.
- Require explicit configured origin and recheck managed-source ownership and enabled state.

## Impact

Changes core models/ports, database adapters, additive migrations, shared contract tests, and the project verify skill. Existing producers remain unchanged. Producer wiring and connected web/Apple views belong to the broader ingestion-outcomes change.
