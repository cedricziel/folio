## 1. Contract
- [x] Inspect pipeline, ledger fencing, scheduler, and shared envelope.
- [x] Define item/attempt identities, bounded events, and integration dependencies.
- [ ] Confirm concrete shared publisher and trigger registry after dependency merges.

## 2. Durable producers
- [ ] Write failing regression `item_outcome_and_outbox_rollback_together`; add both database migrations and transactional outcome persistence.
- [ ] Write failing `repeated_observation_emits_once` and `stale_attempt_cannot_publish`; persist item/attempt identity and keep content deduplication intact.
- [ ] Write failing `folder_retry_retains_item_identity` and `email_attachment_replay_emits_once`; integrate both adapters without changing external cleanup.
- [ ] Write failing `source_failure_recovery_emit_only_on_transition`; persist health changes atomically with resolved vault context.
- [ ] Write failing `skip_veto_precedes_import` and `outcome_payload_excludes_private_input`; retain veto and sanitize event data.

## 3. User interfaces
- [x] Specify exact JSON outcome route and failure statuses; write failing cross-vault and pagination tests, implement API, regenerate OpenAPI and Swift.
- [x] Write failing web source outcome display regression; implement recent outcomes and source health using tokens.
- [ ] Write failing native outcome loading/error/retry tests; implement source outcome views using tokens.
- [ ] Integrate shared trigger registry and both builders; test rejection of SkipImport and document actions on non-document outcomes.

## 4. Delivery
- [ ] Validate OpenSpec strictly; run make format, make lint, and make test.
- [ ] Run isolated project verify, including PostgreSQL transactions, web and native checks. Verify fresh migrations and upgrades across dependency landing orders using the reserved 20261102171000–20261102171999 range.
- [ ] Open reviewable PRs; resolve CI/review/conflicts and merge with required approvals.
- [ ] Verify all merges, archive OpenSpec, disable monitoring, and self-archive chat.

## Independent implementation progress
- [x] Add bounded outcome codes, source health transition classification, and opaque item keys; six core regressions pass.
- [x] Pass stable observations through both source adapters; folder restart/replacement regression and localhost email retry/UIDVALIDITY regression pass.
- [x] Add matching reserved SQLite/PostgreSQL schema; SQLite constraints and expired-lease fencing tests pass.
- [x] Verify PostgreSQL claim/read implementation with an isolated container.
- [x] Add native outcome component and state model; six isolated regressions pass and component compiles with shared theme.
- [x] Connect read-only API, web page, OpenAPI, and generated native adapter.
- [ ] Integrate transactional completion and health publication with the merged shared publisher; adapters currently retain existing ingestion behavior.

## Landed increments and current verification
- [x] Merge stable item identity as #269, commit 86f8110; 621 workspace tests, required PostgreSQL fixtures, lint, required CI, and end-to-end CI passed.
- [x] Observe the missing transactional-writer regression fail, then pass all eight outcome persistence tests on SQLite and PostgreSQL, including concurrent replay and injected outbox rollback.
- [x] Pass the generated native outcome adapter regression. The first full native run had ten attachment-title expectation failures already fixed on main; that fix is now integrated.
- [x] Verify post-merge identity deduplication, trusted lineage, content-ledger fencing, observed pipeline integration, and folder/email recovery after outcome commit failure.
- [x] Add explicit managed/configured source context and verify managed ownership and enabled state in claim/completion transactions on both databases. Recheck soft-deletion once the foundation source lifecycle lands.
- [x] Implement and verify atomic source status/failure/recovery publication.
- [ ] Finish connected native UI and workflow integration (tracked below).

Completed task-owned build outputs were removed after host disk pressure. No build remains active. Coordinate the next bounded Rust verification and full Apple build with the orchestrator; retain jobs <= 2 and disable debug symbols/incremental compilation for fresh Rust targets.

## Verification on 2026-11-03
- [x] Integrate merged trigger registry #280 through cc9ebff without conflicts. Engine/builder wiring remains pending.
- [x] Pass 202 isolated persistence/core tests, including existing-ledger upgrades, repeated migration, no backfill, source health, and ownership on SQLite/PostgreSQL.
- [x] Pass folder/email recovery regressions after injected outcome publication failure, preserving one document and one imported outcome.
- [ ] Run lint and full project verification before publishing the staged persistence increment.

Current bounded builds are complete. Task-owned Rust output was removed to release 11 GiB free capacity for queued processing verification. Logs and all source work remain. No new full build until the queued work has capacity.

## Merged persistence
- [x] Merge #293 at ff7c20e with atomic outcome/content-ledger/outbox commits, source health transitions, explicit source origin, both migrations, and regression coverage.
- [x] Archive the focused ingestion-outcome-persistence spec after verification; retain this broader change for producer and connected UI completion.
- [x] Verify restored producer/UI work against merged persistence and subsequent shared contracts.

## Producer retry preparation
- [x] Run new failed/cancelled folder/email pipeline regressions after the coordinated build queue releases capacity.
- [ ] Integrate workflow authorization for owner-only non-document events; current engine filters them out. Preserve Owners audience.

## Producer and server UI completion (2026-11-03)
- [x] Merge origin/main; regenerate OpenAPI. Pass 26 ingest pipeline regressions, the scheduler regression (managed-vault routing, failure/recovery health), and 75 folio-api source/vault/route/workflow tests on SQLite. fmt and clippy are clean for the touched crates.
- [ ] Native outcome views ship in a separate Apple PR; verified in CI only (local Swift builds paused for disk pressure).
- [ ] Workflow builder/engine integration for owner-only non-document events remains open.
