## 1. Recoverable document lifecycle
- [x] Implement repository lifecycle and worker retry behavior, with repository contract tests for retained files, restore, scope, purge conflict, and failure recovery.
- [x] Integrate JSON and HTML routes with `deletion_keeps_files_until_explicit_purge_and_restore_recovers_document` and permission regression tests.
- [x] Add the Trash interface and verify restore and separately confirmed permanent deletion through browser tests.
- [x] Update events, OpenAPI, and Apple wording; verify client decoding and event isolation.

## 2. Verification and delivery
- [x] Run make format, make lint, make test, database contracts, browser journeys, and relevant Apple verification.
- [x] Validate and archive this change after implementation and checks pass.
