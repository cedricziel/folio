## 1. Contract
- [x] Inventory persisted entities and document lifecycle exceptions.
- [x] Validate the OpenSpec change and publish the dependency contract.

## 2. Persistence and attribution
- [ ] Add failing repository contract tests for actor metadata, live-name reuse, restore conflicts, retained references, and cross-vault rejection.
- [ ] Add forward-only SQLite and PostgreSQL migrations and implement lifecycle-aware writes.
- [x] Add failing HTTP and MCP tests for spoofed actors and concurrent identity isolation, then implement authenticated mutation context.
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


## 6. Classification delivery
- [x] Implement and verify taxonomy lifecycle metadata and retained references on SQLite and PostgreSQL.
- [x] Verify web trash, name reuse, restore conflicts, vault isolation, and viewer restrictions in a disposable browser instance.
- [x] Regenerate OpenAPI and verify native tag compatibility with the Swift package suite.
- [ ] Deliver native classification management after its isolated UI verification.

## Vault increment
- [ ] Verify both database migrations, scoped metadata, atomic owner creation, retained references, no-op/rollback suppression, and permanent vault/membership audit facts.
- [ ] Verify protected defaults, current-owner recovery, viewer/admin isolation, workflow cancellation/fencing, and managed/configured source activity checks.
- [ ] Verify usable web owner controls and desktop/mobile lifecycle regression coverage.
- [ ] Regenerate API/client contracts and implement Apple metadata, owner management, recovery, and isolated native regressions before completing the full lifecycle change.
