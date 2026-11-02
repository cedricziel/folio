## 1. Contract
- [x] Inventory persisted entities and document lifecycle exceptions.
- [x] Validate the OpenSpec change and publish the dependency contract.

## 2. Persistence and attribution
- [ ] Add failing repository contract tests for actor metadata, live-name reuse, restore conflicts, retained references, and cross-vault rejection.
- [ ] Add forward-only SQLite and PostgreSQL migrations and implement lifecycle-aware writes.
- [ ] Add failing HTTP and MCP tests for spoofed actors and concurrent identity isolation, then implement authenticated mutation context.
- [ ] Preserve document trash, move, worker, workflow, and ingestion behavior with regression coverage.

## 3. Interfaces
- [ ] Add JSON/HTML lifecycle routes and OpenAPI coverage, then regenerate the Swift client.
- [ ] Add web lifecycle state and controls with a failing isolated browser regression.
- [ ] Add Apple lifecycle state and controls with failing service/view-model regressions.

## 4. Domain event amendment
- [ ] Add failing contract tests for versioned envelopes, actor/vault/subject identity, legacy payloads, rollback/no-op suppression, retry identity, and correlation/causation.
- [ ] Extend the existing outbox and publish lifecycle transitions transactionally on both databases.
- [ ] Integrate document/non-document trigger contracts with the workflow slice and verify both web and Apple trigger selection.

## 5. Verification and delivery
- [ ] Run project-local verify in disposable environments and update covered behavior.
- [ ] Run make format, make lint, and make test.
- [ ] Archive completed specifications, merge reviewable PRs, and verify their merged status.
