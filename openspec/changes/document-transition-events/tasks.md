## 1. Contract
- [x] Inspect ownership and publish scoped event contract.
- [ ] Reconcile foundation append API and workflow registry after merge.

## 2. Transactional producers
- [x] Add failing delta tests for nullable IDs, title changes, duplicate tags, and equivalent reordered sets; implement pure delta derivation.
- [ ] Add failing SQLite/PostgreSQL repository contracts for update/add/remove/set ingress, rollback, no-op, and immutable context; append transitions atomically.
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

Initial delta suite failed with missing `folio_core::document_transitions`, then all four tests passed after implementation. After fast-forwarding the task branch to `2da7bff`, `cargo test -p folio-core` passed all 50 tests and `cargo clippy -p folio-core --all-targets -- -D warnings` passed. `make format`, `git diff --check`, and strict OpenSpec validation passed. These scoped checks do not replace the pending full workspace and client verification. The pure delta API is committed as `17e025a` after scoped checks; full workspace lint is running before merge.

Task worktree: `/private/tmp/folio-document-events`, branch `codex/document-events`. Dependency monitoring: `finish-document-transition-events`. The first PR contains only the pure delta API and this contract; publication is in progress. Foundation envelope/transactional publisher and workflow registry are not merged. Next: inspect merged dependencies, rebase this isolated branch, integrate producer deltas and trigger controls, then run full verification and ship.
