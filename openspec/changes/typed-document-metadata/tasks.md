## 1. Contract and dependencies

- [x] Inspect existing models, API authorization, workspace guidance, and dependency chats.
- [x] Define typed values, exact amounts, lifecycle, uniqueness, references, and both client experiences.
- [ ] Validate OpenSpec and confirm merged foundation and logical document contract before shared implementation.

## 2. Domain and persistence

- [ ] Write failing `metadata_validation` tests for exact decimals, false/null, date validity, currency, URL, and type mismatches; implement domain types and validation.
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

- [ ] Update the project verify skill with new meaningful regressions.
- [ ] Run make format, make lint, make test, required PostgreSQL contracts, isolated Apple tests/build, and isolated web checks.
- [ ] Validate changed specs, archive the completed change, and open reviewable PRs after foundation integration.
- [ ] Resolve CI, review findings, and conflicts; verify every deliverable merged.
- [ ] Disable monitoring, report merged PRs and verification, and archive this chat.

## 6. Workflow event amendment

- [ ] Align event envelope and trigger dependency contracts with foundation and workflow slices.
- [ ] Write failing outbox contract tests for committed metadata transitions, no-op suppression, rollback, retry identity, purge clearing, vault scope, and causation; publish events in mutation transactions.
- [ ] Verify metadata triggers and typed conditions are selectable in web and Apple workflows; reject document-only actions for field/option events.
- [ ] Include event integration, CI, and merged PR verification in completion criteria.
