## 1. Contract and core builder

- [x] 1.1 Confirm merged foundation contracts and record migration names.
- [x] 1.2 Implement bounded condition groups after eight failing regressions; verify legacy roundtrip, snapshot matching, and recursive OpenAPI schema.
- [x] 1.3 Write failing multiple-trigger compatibility tests; implement normalized definitions.
- [x] 1.4 Write failing ordered-action retry and cross-vault reference tests; implement rename, remove tags, clear classifications, and trash.
- [x] 1.5 Write failing stale-definition and revoked-authority tests; implement cancellation without rewriting snapshots.
- [x] 1.6 Write failing web builder roundtrip/order tests and native builder service tests; implement both builders and regenerate OpenAPI/Swift.

## 2. Durable scheduling

- [x] 2.1 Write failing concurrent-claim, restart, edit/disable, and bounded-catch-up tests on both databases; add new migrations and scheduler ports/adapters.
- [x] 2.2 Write failing scheduled/recurring builder tests; implement schedule controls on web and Apple.

## 3. Cross-feature integration

- [x] 3.1 Record merged metadata and document lifecycle contracts.
- [x] 3.2 Write failing typed metadata, missing value, document date offset, review/version event, and workflow-loop regressions; integrate services and snapshots.
- [x] 3.3 Write failing web/native typed-field and date/review action tests; expose integrated controls.

## 4. Shared domain event amendment

- [x] 4.1 Record the foundation envelope, publisher, visibility, and causation contracts after merge.
- [x] 4.2 Add the typed trigger registry and validate multi-trigger action compatibility, duplicate registrations, unknown selections, and condition catalogs.
- [x] 4.3 Implement shared envelope-plus-legacy candidate resolution and transactional workflow/mutation deduplication; cover document.purged plus Deleted compatibility, multi-selection, distinct workflows, distinct mutations sharing correlation, concurrent delivery, and legacy rows without envelopes.
- [x] 4.4 Write failing private-event visibility, deleted-subject, vault-move, rollback/no-op, and causation-loop tests across the integrated slices.
- [x] 4.5 Write failing web/native trigger selection and condition-data tests; expose all supported domain events in both builders.
- [x] 4.6 Verify workflow definition lifecycle publication and final cross-slice event integration before merge completion.
- [x] 4.7 Integrate all six document_note/document_comment transitions in the catalog, dispatch, condition data, and web/Apple builders; consume actual SQLite/PostgreSQL producer envelopes in regressions, enforce current parent authorization for document actions, and verify body exclusion, replay deduplication, moved parents, revoked access, and recursion suppression. Integrate after #291 using the merged #277 producer contract. Producer publication alone does not complete this requirement.

## 5. Verification and delivery

- [x] 5.1 Run make format, make lint, make test, isolated SQLite/PostgreSQL contracts, project verify, and native build/tests. Update verify coverage.
- [x] 5.2 Validate OpenSpec, archive completed change, and open reviewable PRs in dependency order.
- [x] 5.3 Resolve CI and review findings and verify the implementation PRs merged.


The Codex heartbeat and chat are retired after this archive is verified on protected main.
