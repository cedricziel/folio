## Implementation

- [x] Persist one-time and recurring clocks and their occurrence identity on both databases.
- [x] Enqueue each occurrence with the captured workflow revision and job in one transaction.
- [x] Wire the server scheduler, JSON API, and web editor.
- [x] Verify concurrency, restart, catch-up, edit and rollback on SQLite and PostgreSQL.
- [x] Regenerate OpenAPI and verify the Swift transport contract.
- [x] Resolve the review finding, verify the repaired scheduler on SQLite and PostgreSQL, and confirm the protected merge.
