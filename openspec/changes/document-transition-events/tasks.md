## 1. Contract
- [x] Inspect ownership and publish scoped event contract.
- [ ] Reconcile foundation append API and workflow registry after merge.

## 2. Transactional producers
- [x] Add failing delta tests for nullable IDs, title changes, duplicate tags, and equivalent reordered sets; implement pure delta derivation.
- [x] Add failing SQLite/PostgreSQL repository contracts for update/add/remove/set ingress, rollback, no-op, and immutable context; append transitions atomically.
- [ ] Add failing move and purge contracts for scoped snapshots, retries, and distinct request/completion; implement lifecycle producers.
- [ ] Add failing HTTP/MCP/workflow/bulk regressions for applicable ingress paths and source-context authorization.

## 3. Workflow controls
- [ ] Add failing trigger/condition and deduplication tests; register specific events through the shared engine.
- [ ] Add web and Apple tests for readable event choices, conditions, and unavailable-subject errors; extend shared controls and regenerate OpenAPI/Swift.

## 4. Ship
- [ ] Run strict OpenSpec validation, make format, make lint, make test, isolated project verify, and relevant Apple checks.
- [ ] Archive specification, open reviewable PRs, address CI/reviews, and verify merges.
- [ ] Disable monitoring and archive this chat only after every deliverable merges.

## Verification record

- [PR #261](https://github.com/cedricziel/folio/pull/261) merged the pure delta API and contract as `3355716`. Core tests, Clippy, strict OpenSpec validation, required workspace CI, embedding builds, end-to-end tests, and both container builds passed.
- Actor #258, ownership #260, envelope #265, and lineage #266 are merged prerequisites. The shared workflow registry is a separate integration.
- SQLite regression contracts first failed on missing title/tag/move/purge events. Metadata producers then passed six SQLite/PostgreSQL contracts, eight repository contracts, and 35 workflow tests with Docker required.
- HTML/API classification forms and MCP metadata edits pass their focused ingress checks. Core lineage and cross-vault batch tests pass. Move/purge producers and workflow controls remain pending.

- [PR #271](https://github.com/cedricziel/folio/pull/271) contains `metadata_envelopes` and transactional metadata producers. Commits `35a3ef2` and `a722013` define the date-event integration boundary. Required CI gates merge.
