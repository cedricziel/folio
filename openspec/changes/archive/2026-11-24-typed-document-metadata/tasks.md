## 1. Contract and dependencies

- [x] Inspect existing models, API authorization, workspace guidance, and dependency chats.
- [x] Define typed values, exact amounts, lifecycle, uniqueness, references, and both client experiences.
- [x] Validate OpenSpec and integrate merged actor #258, ownership #260, and transactional event #265 prerequisites. Logical document links continue to use documents.id.

## 2. Domain and persistence

- [x] Merge field/option model PR #267 as c5d9a72621709a4be87e243f9953b5fd67d56144. Eight test-first `metadata_fields` regressions, all 76 core tests, core clippy, and workspace make lint pass.

- [x] Write failing `metadata_validation` tests for exact decimals, false/null, date validity, currency, URL, and type mismatches; implement domain types and validation. All 22 regressions and 46 existing core tests pass with the openapi feature. Workspace make lint and make format also pass.
- [x] Write failing shared `metadata_repository_contract` tests for audit actors, live uniqueness, option identity, atomic patches, concurrent edits, vault isolation, trash/purge, and move rejection; implement new SQLite/PostgreSQL migrations and adapters.
- [x] Merge the independently usable typed persistence in #310. Shared SQLite and PostgreSQL contracts cover transactional writes, vault scope, move guards, and purge clearing.

## 3. API and web

- [x] Merge scoped field/option JSON lifecycle and OpenAPI as #521. All nine metadata router tests, workspace CI, end-to-end, fastembed, Paperless, and architecture builds passed.
- [x] Merge scoped document metadata reads and atomic revision-checked updates in #528. Local API, vault, token, SQLite and required PostgreSQL tests passed. Hosted workspace CI confirmed explicit-null OpenAPI, E2E and architecture builds.

- [x] Verify JSON and HTML routes for vault roles, contributor ownership, revoked grants, hidden targets, validation errors, and CSRF. The #521 and #528 API contracts and #535 and #539 web checks passed.
- [x] Merge exact metadata search as #531. Both required backend regressions, the metadata router and OpenAPI suites, workspace CI, end-to-end tests, and both architecture builds passed.
- [x] Merge web field management and typed document editing in #535 and #539. Verify option paging, retained drafts, Viewer access, exact decimal strings, and logical link navigation in disposable router and browser fixtures.
- [x] Regenerate OpenAPI and check schema equality. The hosted #528 MCP HTTP, OAuth, skills, tools, and vocabulary suites passed.

## 4. Apple

- [x] Verify the generated client and view model with mock requests and isolated Swift tests for vault scope, exact decimal and null round trips, stale edits, and cancellation. The stale edit and cancellation regressions failed when the corresponding guards were removed in disposable test copies and passed with the shipped model.
- [x] Merge the native manager, typed editor and display, required states, and logical link routing in #547. The isolated iPhone fixture passed exact decimal editing and viewer link navigation. The #556 run also passed both cases on iPhone Duo.

## 5. Verification and delivery

- [x] Merge the independent validation increment as PR #259, merge commit d35618363509284c41bed910aca362d07c7f158b. CI, end-to-end, fastembed, and title lint passed.

- [x] Update the project verify skill with the shared typed-field repository contract; extend it as later metadata behaviors land.
- [x] Run workspace format, lint and tests, required SQLite and PostgreSQL contracts, isolated browser checks, and the Apple package, build, and navigation checks. See the verification links below.
- [x] Validate the change, publish the typed metadata capability to the main specs, and archive this completed change.
- [x] Audit CI and review evidence and confirm every implementation PR is merged and reachable from main.
Stop delivery monitoring and archive the delivery chat after the archive PR merges.

## 6. Workflow event amendment

- [x] Align and merge the event budget, bounded replay, and workflow dispatch in #296, #520, and #543.
- [x] Verify the outbox contracts in #310, #528, and #543. Tests cover transaction rollback, unchanged values, purge clearing, vault scope, immutable occurrence data, repeat dispatch, and lineage.
- [x] Verify metadata triggers and exact typed conditions in the web and Apple workflow editors. #543 and #551 reject document-only actions for field and option events. #556 passed exact decimal and Any type saves on iPhone and iPhone Duo.
- [x] Verify merged event integration and record the runtime and CI results for the full slice.

## Merged implementation and verification

- Types, field and option lifecycle, and typed persistence landed in
  [#259](https://github.com/cedricziel/folio/pull/259),
  [#267](https://github.com/cedricziel/folio/pull/267),
  [#285](https://github.com/cedricziel/folio/pull/285),
  [#294](https://github.com/cedricziel/folio/pull/294), and
  [#310](https://github.com/cedricziel/folio/pull/310).
  The shared SQLite and PostgreSQL contracts checked exact values, uniqueness,
  vault scope, concurrent edits, lifecycle, moves, and purge.
- The envelope and bounded replay landed in
  [#296](https://github.com/cedricziel/folio/pull/296) and
  [#520](https://github.com/cedricziel/folio/pull/520). Repository tests verify
  complete event replay at the byte limit. The #528 router test verifies atomic
  rejection of an oversized transition.
- JSON, exact filtering, and web editing landed in
  [#521](https://github.com/cedricziel/folio/pull/521),
  [#528](https://github.com/cedricziel/folio/pull/528),
  [#531](https://github.com/cedricziel/folio/pull/531),
  [#535](https://github.com/cedricziel/folio/pull/535), and
  [#539](https://github.com/cedricziel/folio/pull/539).
  The archived base also contains the composable saved-view filter in
  [#542](https://github.com/cedricziel/folio/pull/542).
- Workflow dispatch and exact web conditions landed in
  [#543](https://github.com/cedricziel/folio/pull/543) and
  [#551](https://github.com/cedricziel/folio/pull/551).
  The hosted and isolated checks verified committed occurrence data, workflow
  conditions, and action compatibility.
- The generated Apple client, field manager and editor, and workflow editor
  landed in [#545](https://github.com/cedricziel/folio/pull/545),
  [#547](https://github.com/cedricziel/folio/pull/547), and
  [#556](https://github.com/cedricziel/folio/pull/556). The final Apple workflow
  run passed the generated wire test, iOS and macOS builds, and the exact
  decimal and Any type cases on both iPhone simulators.
  [Apple verification](https://github.com/cedricziel/folio/actions/runs/45232491751/job/138084018885)
