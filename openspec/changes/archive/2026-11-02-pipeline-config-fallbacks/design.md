## Context

See proposal.md. Historical migrations seed pipeline settings. Existing values cannot be distinguished safely from deliberate user choices, so upgrades must retain all of them.

## Decisions

- Reuse the core OCR, embedding, and processing settings types in startup config, with field-level defaults for partial sections. Overlay database rows on a configured pipeline snapshot and retain the existing API-key fallback behavior.
- Render the effective snapshot on the existing HTML settings page. No JSON or generated-client changes.
- Migration 000 records whether initialization began without migration history. Its low version lets it distinguish fresh installations before historical seed migrations run; on upgrades it preserves every setting. After migrations succeed, atomically remove fresh pipeline defaults and clear the marker. A restart retries unfinished cleanup while holding the backend migration lock.
- SQLite needs an exclusive connection lock across migrations because the vault migration explicitly manages its own transaction. An outer transaction would break that migration. Restore normal locking after initialization, including error cleanup. SQLite documents connection locking in [PRAGMA locking_mode](https://www.sqlite.org/pragma.html#pragma_locking_mode). PostgreSQL uses its existing advisory migration lock.

## Risks

- Accidentally treating an upgrade as fresh could discard settings. Tests cover both backends, repeated initialization, and preservation of existing default-valued overrides.
- Initialization holds a database lock until migration and default cleanup finish. Existing per-migration transaction behavior remains unchanged.

## Deployment

A new migration adds the single-row pipeline initialization marker. In-memory SQLite uses one persistent pooled connection to serialize initialization and retain its database. New installations inherit startup values until a user saves settings. Upgrades retain their database overrides, including values equal to built-in defaults.
