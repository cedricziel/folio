## 1. Contracts and prerequisites
- [x] 1.1 Inspect shared importer, original keys, processing writes, and document lifecycle.
- [ ] 1.2 Validate specs and reconcile merged foundation/date-review contracts before integration.

## 2. Durable files
- [x] 2.0 Add failing unit tests and implement byte-derived FileIdentity plus collision-separated revision/artifact keys; importer persistence and immutability enforcement remain in 2.1.
- [ ] 2.1 Write failing importer/storage tests covering every upload entry, byte identity, retry, and late purge; implement immutable staging and identity.
- [ ] 2.2 Write failing SQLite/PostgreSQL migration tests for first revision, missing originals, resumable verification, and incomplete uploads; implement backfill.
- [ ] 2.3 Write failing web/Swift detail tests for file identity and verification states; expose file details through JSON/OpenAPI/MCP and both clients.

## 3. Revision operations
- [ ] 3.1 Write failing shared contracts for concurrent append, monotonic numbering, idempotency, and restore-as-new; implement repository operations.
- [ ] 3.2 Write failing worker/search tests for late OCR, chunk isolation, derivative retention, and revision-bound queues; implement processing fences.
- [ ] 3.3 Write failing route tests for historical reads, replacement, restore, payload validation, and role/vault denial; expose APIs and regenerate the Swift client.
- [ ] 3.4 Write failing lifecycle tests for trash/restore, full purge with late writes, and vault move during processing; implement ownership updates and cleanup.

## 4. Both clients
- [ ] 4.1 Write failing web acceptance tests for history, replacement, download, restore, and conflict; implement the full detail flow.
- [ ] 4.2 Write failing Swift adapter/view-model tests for history, replacement, restore, stale previews, and conflict; implement native detail flow.

## 5. Verification and merge
- [ ] 5.1 Run isolated project verify, real PostgreSQL/SQLite contracts, Apple tests/build, and web acceptance checks; update verify guidance for revisions.
- [ ] 5.2 Run make format, make lint, make test, and strict OpenSpec validation; archive completed specs in the final implementation commit.
- [ ] 5.3 Resolve PR CI/reviews/conflicts until all deliverables merge; verify merged status, disable monitoring, and archive this chat only then.

## 6. Workflow events
- [ ] 6.1 Reconcile the foundation event envelope and shared workflow trigger registry before integration.
- [ ] 6.2 Write failing SQLite/PostgreSQL outbox tests for committed transitions, rollback, no-op, retries, actor/correlation, and moved/deleted subjects; implement event definitions and transactional emission.
- [ ] 6.3 Write failing workflow integration tests for the new trigger conditions and recursion protection; verify understandable selectors and condition fields in web and Apple with the workflow slice.
