## 1. Contract and dependency
- [x] 1.1 Inspect existing workspace review, import, storage, and UI contracts.
- [x] 1.2 Validate this proposal and reconcile the foundation's merged actor context before schema/API edits.

## 2. Date and review behavior
- [x] 2.0 Add failing pure-domain date and review regression tests, then implement strict dates and idempotent review decisions without shared schema changes.
- [x] 2.1 Write failing shared repository tests for leap dates, null/omission, legacy backfill, idempotent review attribution, and processing independence; implement additive migrations and ports.
- [ ] 2.2 Write failing route tests for review/date edits, actor spoofing, trash, role and vault denial; implement JSON and HTML actions plus OpenAPI.
- [ ] 2.3 Write failing repository and route tests for combined review/date filters, null dates, and changed cursors; implement filtering.

## 3. Both clients
- [x] 3.0 Add failing Swift date regressions and implement the date model; editor integration remains in 3.2.
- [ ] 3.1 Write failing web acceptance tests for date clear, review/unreview, filters, and failed writes; implement token-based detail and browse controls.
- [ ] 3.2 Write failing Swift date round-trip, review, filter, and view-model error tests; regenerate the client and implement native detail/browse controls.
- [ ] 3.3 Verify MCP reads preserve metadata and expose review state without accepting caller-supplied audit identity.

## 4. Verification and delivery
- [ ] 4.1 Run isolated project verify, PostgreSQL/SQLite contracts, Apple tests/build, and web acceptance checks; update covered behavior in the verify skill.
- [ ] 4.2 Run make format, make lint, make test, and strict OpenSpec validation; archive completed specs in the final implementation commit.
- [ ] 4.3 Open reviewable PRs, resolve checks/reviews/conflicts, and verify merge before starting dependent revision integration.

## 6. Workflow events
- [x] 6.1 Reconcile the foundation event envelope and shared workflow trigger registry before integration.
- [ ] 6.2 Write failing SQLite/PostgreSQL outbox tests for committed transitions, rollback, no-op, retries, actor/correlation, and moved/deleted subjects; implement event definitions and transactional emission.
- [ ] 6.3 Write failing workflow integration tests for the new trigger conditions and recursion protection; verify understandable selectors and condition fields in web and Apple with the workflow slice.
