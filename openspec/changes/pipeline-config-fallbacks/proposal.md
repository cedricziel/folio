## Why

Issue #46 requires pipeline configuration through environment variables. The typed config omits OCR, embedding, and processing keys, and migration-seeded database defaults would hide their startup values on a fresh installation.

## What Changes

- Add existing pipeline keys to typed startup configuration and the environment registry.
- Merge saved database settings over startup configuration in the worker and HTML settings page.
- On fresh databases only, remove migration-seeded pipeline defaults while migration initialization remains locked. Preserve every existing setting on upgrades.

## Capabilities

### Modified Capabilities

- `settings`: Pipeline startup configuration follows defaults < config file < environment < saved database values.

## Impact

Touches folio-core settings models, folio-infra configuration and database initialization, folio-api's existing HTML settings route, and binary wiring. No JSON endpoint, OpenAPI, Swift client, or database schema changes. Existing keys remain in both startup configuration and the settings table; saved database values win.
