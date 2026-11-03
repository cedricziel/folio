## 1. Contract and dependencies

- [x] Inspect existing models, API authorization, workspace guidance, and dependency chats.
- [x] Define typed values, exact amounts, lifecycle, uniqueness, references, and both client experiences.
- [x] Validate OpenSpec and integrate merged actor #258, ownership #260, and transactional event #265 prerequisites. Logical document links continue to use documents.id.

## 2. Domain and persistence

- [x] Merge field/option model PR #267 as c5d9a72621709a4be87e243f9953b5fd67d56144. Eight test-first `metadata_fields` regressions, all 76 core tests, core clippy, and workspace make lint pass.

- [x] Write failing `metadata_validation` tests for exact decimals, false/null, date validity, currency, URL, and type mismatches; implement domain types and validation. All 22 regressions and 46 existing core tests pass with the openapi feature. Workspace make lint and make format also pass.
- [ ] Write failing shared `metadata_repository_contract` tests for audit actors, live uniqueness, option identity, atomic patches, concurrent edits, vault isolation, trash/purge, and move rejection; implement new SQLite/PostgreSQL migrations and adapters.
- [ ] Run relevant checks and commit the independently usable persistence unit.

## 3. API and web

- [ ] Write failing router tests for manager/editor/viewer permissions, contributor ownership, revoked membership, cross-vault targets, errors, and CSRF; implement documented JSON and HTML routes.
- [ ] Write failing search tests for canonical equality, bounds, cursor binding, and retired fields; implement exact metadata filtering.
- [ ] Add web management and typed document forms with retained failed drafts; verify create, edit, retirement, restore, and link navigation through isolated Playwright fixtures.
- [ ] Regenerate OpenAPI and verify MCP compatibility using the existing protocol suite.

## 4. Apple

- [ ] Write failing mock-service tests for typed round trips, exact amounts, conflicts, scoped document links, and cancellation; implement service and view models using the generated client.
- [ ] Add native management, typed editor/display, required-field states, and logical link navigation; verify viewer restrictions and narrow layouts in an isolated fixture.

## 5. Verification and delivery

- [x] Merge the independent validation increment as PR #259, merge commit d35618363509284c41bed910aca362d07c7f158b. CI, e2e, fastembed, and title lint passed. The full metadata feature remains incomplete.

- [x] Update the project verify skill with the shared typed-field repository contract; extend it as later metadata behaviors land.
- [ ] Run make format, make lint, make test, required PostgreSQL contracts, isolated Apple tests/build, and isolated web checks.
- [ ] Validate changed specs, archive the completed change, and open reviewable PRs after foundation integration.
- [ ] Resolve CI, review findings, and conflicts; verify every deliverable merged.
- [ ] Disable monitoring, report merged PRs and verification, and archive this chat.

## 6. Workflow event amendment

- [ ] Align event envelope and trigger dependency contracts with foundation and workflow slices.
- [ ] Write failing outbox contract tests for committed metadata transitions, no-op suppression, rollback, retry identity, purge clearing, vault scope, and causation; publish events in mutation transactions.
- [ ] Verify metadata triggers and typed conditions are selectable in web and Apple workflows; reject document-only actions for field/option events.
- [ ] Include event integration, CI, and merged PR verification in completion criteria.

## Current verification checkpoint

- In progress: document-value persistence on `codex/metadata-values`, based on merged option storage. The maximum-length long-text regression failed against the 64 KiB envelope bound. A scoped 1 MiB budget proposal now preserves complete snapshots only for vault-visible document.metadata.changed events. All 102 core tests pass with OpenAPI after rebasing current main, including boundary, decode, audience, exact event-name, and target regressions. Scoped lint passes; the shared payload contract is ready for foundation/workflow review.
- Option storage PR #294 merged as 0038e97d1713c8718167d0d97be09dca5d694c4d. Required SQLite/PostgreSQL field/option contracts, scoped lint, workspace CI, E2E, fastembed, and title lint passed. Field storage #285 and snapshot fix #286 remain integrated.
- The orchestrator assigned the Rust slot after dates/review-state. Verification finished with two jobs, no incremental compilation, and zero debug symbols. Logs: `/private/tmp/folio-metadata-event-{red,test,lint,integrated}.log`. No build remains active; `/private/tmp/folio-metadata-event-target` was removed and the slot released. Workflow event-condition verification is next. No Apple build slot is assigned.
