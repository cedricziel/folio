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

- In progress: stable select-option persistence on `codex/metadata-options`. The shared contract failed on missing port/adapters before implementation. SQLite and required disposable PostgreSQL contracts now pass for stable identity, audit actors, max position, field/vault isolation, retirement/restore, no-ops, concurrency, pagination, parent-kind integrity, and transactional outbox rollback. Scoped core/infra clippy passes.
- Field storage PR #285 merged as f48a63820e05fde0ce247a513900591d10a76e19. Required local SQLite/PostgreSQL tests, trusted lineage checks, scoped lint, workspace CI, and fastembed pass. The pre-existing desktop screenshot fix PR #286 is merged as 020f4cbd3408f2a6ae41dee88789041b37289e8b with successful CI and E2E.
- The orchestrator assigned the bounded Rust slot after ingestion released it. Both field and option contracts pass after rebasing current main; evidence: `/private/tmp/folio-metadata-options-integrated.log`. Build settings were two jobs, incremental disabled, zero debug symbols. No build remains active. The completed `/private/tmp/folio-metadata-options-target` is being reclaimed to release capacity; dates/review-state is next. No Apple build slot is assigned.
