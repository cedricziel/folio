## Context

See proposal.md. Historical migrations seed pipeline settings. Existing values cannot be distinguished safely from deliberate user choices, so upgrades must retain all of them.

## Decisions

- Reuse the core OCR, embedding, and processing settings types in startup config, with field-level defaults for partial sections. Overlay database rows on a configured pipeline snapshot and retain the existing API-key fallback behavior.
- Render the effective snapshot on the existing HTML settings page. No JSON or generated-client changes.
- Detect a database with no migration history while holding a backend migration lock. After its migrations succeed, remove only pipeline defaults from that fresh database. Existing databases retain every row.
- SQLite needs an exclusive connection lock across migrations because the vault migration explicitly manages its own transaction. An outer transaction would break that migration. Restore normal locking after initialization, including error cleanup. SQLite documents connection locking in [PRAGMA locking_mode](https://www.sqlite.org/pragma.html#pragma_locking_mode). PostgreSQL uses its existing advisory migration lock.

## Risks

- Accidentally treating an upgrade as fresh could discard settings. Tests cover both backends, repeated initialization, and preservation of existing default-valued overrides.
- Initialization holds a database lock until migration and default cleanup finish. Existing per-migration transaction behavior remains unchanged.

## Deployment

No schema change or new migration. New installations inherit startup values until a user saves settings. Upgrades retain their database overrides, including values equal to built-in defaults.
