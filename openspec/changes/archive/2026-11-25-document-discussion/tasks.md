## 1. Contract
- [x] Inspect the existing document, vault, web, and Apple architecture.
- [x] Define distinct note/comment UX and a shared entry contract.
- [x] Validate this OpenSpec change and align with the published foundation contract.
- [x] Integrate merged actor #258, ownership #260, and envelope/outbox #265; verify the exported actor and transaction-scoped append APIs.

## 2. Persistence and API
- [x] Write failing core regressions for text validation, authorship/moderation, wire kinds, and bounded scoped continuations; implement the entry model.
- [x] Verify entry models with OpenAPI enabled and run core lint/tests: 51 tests pass; all-target core clippy passes with warnings denied.
- [x] Write failing shared repository regressions for scoped CRUD, pagination, soft deletion, document moves, and purge; implement both adapters and additive migrations in the reserved 20261102163000–20261102163999 range. Check main for collisions and test upgrades across dependency merge orders.
- [x] Write failing router tests for author editing, moderation, spoofed actors, viewer denial, revoked membership, malformed cursors, and cross-vault IDs; implement JSON routes.
- [x] Generate OpenAPI from the four entry operations; five schema tests pass.
- [x] Verify generated Swift client compilation: isolated FolioKit build and 42 focused tests pass.

- [x] Verify atomic creation receipts with concurrent matching keys on SQLite and PostgreSQL, request tests for conflicting payloads, revoked access and deleted-entry retries, and lost-response draft recovery in both clients.

## 3. Workflow events
- [x] Align the design with foundation DomainEventEnvelope v1 and transaction-scoped append API.
- [x] Reconcile merged envelope signatures from #265.
- [x] Integrate the six note and comment triggers into the server catalog and the web and Apple editors. [Server and web #522](https://github.com/cedricziel/folio/pull/522), [Apple #572](https://github.com/cedricziel/folio/pull/572).
- [x] Write failing transaction tests for rollback, no-op, idempotent retries, actor attribution, and body-free payloads; implement six durable entry events.
- [x] Verify that vault-owned workflows keep running after creator revocation while the former member loses workflow and run access. Verify document moves, recursion protection, and web trigger selection. [Dispatch and web CI](https://github.com/cedricziel/folio/actions/runs/45063392537/job/137334091129), [Revocation CI](https://github.com/cedricziel/folio/actions/runs/45064878860/job/137338692689).
- [x] Verify that all six note and comment choices round-trip through the generated Apple client. Verify that note and comment triggers stay selected when the iPhone editor reopens after saving. [Swift catalog test](https://github.com/cedricziel/folio/actions/runs/45345198889/job/138277972326), [iPhone save-and-reopen test](https://github.com/cedricziel/folio/actions/runs/45490503204/job/138787290614).

## 4. Usable clients
- [x] Write failing web regressions for CRUD, failed drafts, pagination, and viewer controls; implement HTML routes and separate note/comment sections.
- [x] Verify lost-response creation retries, CRUD, and accessibility at desktop/mobile widths in the disposable browser fixture.
- [x] Write failing native state/service tests for CRUD, permission capabilities, failed drafts, and pagination; implement Apple discussion UI.
- [x] Verify native app and navigation CI, including discussion CRUD and lost-response recovery. Both discussion navigation tests passed before [#292](https://github.com/cedricziel/folio/pull/292) merged ([Apple CI](https://github.com/cedricziel/folio/actions/runs/41648702209/job/125960794843)).

## 5. Delivery
- [x] Pass formatting, lint, and tests in required CI. Verify picker behavior in the isolated Classification UI fixture and update the Apple verification guide. [Required CI](https://github.com/cedricziel/folio/actions/runs/45490503226/job/138787229156), [iPhone CI](https://github.com/cedricziel/folio/actions/runs/45490503204/job/138787290614).
- [x] Sync the document-discussion requirements to the main spec and archive this change.
- [x] Verify the implementation and regression PRs are merged. [API #283](https://github.com/cedricziel/folio/pull/283), [Web #287](https://github.com/cedricziel/folio/pull/287), [Apple #292](https://github.com/cedricziel/folio/pull/292), [Revocation #526](https://github.com/cedricziel/folio/pull/526), [Apple regression #614](https://github.com/cedricziel/folio/pull/614) at `41b979b1`.
