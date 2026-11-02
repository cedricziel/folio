# Document transition events

## Why

Generic document updates hide classification and title changes. Vault moves only create audit entries, purge requests reuse trash events, and workflows ignore trash and restore.

## What changes

Publish precise transactional document transitions through the existing versioned outbox. Add readable event choices and before/after conditions to the shared web and Apple workflow builders. Preserve generic update compatibility and immutable occurrence context.

## Capabilities

- Modified `document-workflows`: transition triggers and conditions.

## Impact

Core delta models and workflow registration, SQLite and PostgreSQL document repositories, existing JSON and HTML workflow APIs, shared web templates, OpenAPI and generated Swift client, and Apple workflow controls. HTTP and MCP document writes retain their request contracts. No independent event bus, settings, or schema migration is planned; use the foundation envelope migration. Date, review, and revision events belong to the revision slice.
