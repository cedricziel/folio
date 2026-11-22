## 1. Contract
- [x] Inventory persisted entities and document lifecycle exceptions.
- [x] Validate the OpenSpec change and publish the dependency contract.

## 2. Persistence and attribution
- [x] Add failing repository contract tests for actor metadata, live-name reuse, restore conflicts, retained references, and cross-vault rejection.
- [x] Add forward-only SQLite and PostgreSQL migrations and implement lifecycle-aware writes.
- [x] Add failing HTTP and MCP tests for spoofed actors and concurrent identity isolation, then implement authenticated mutation context.
- [x] Preserve document trash, move, worker, workflow, and ingestion behavior with regression coverage.

## 3. Interfaces
- [x] Add JSON/HTML lifecycle routes and OpenAPI coverage, then regenerate the Swift client.
- [x] Add web lifecycle state and controls with a failing isolated browser regression.
- [x] Add Apple lifecycle state and controls with failing service/view-model regressions.

## 4. Domain event amendment
- [x] Add failing contract tests for versioned envelopes, actor/vault/subject identity, legacy payloads, rollback/no-op suppression, retry identity, and correlation/causation.
- [x] Extend the existing outbox and publish lifecycle transitions transactionally on both databases.
- [ ] Integrate document/non-document trigger contracts with the workflow slice and verify both web and Apple trigger selection.

## 5. Verification and delivery
- [x] Run project-local verify in disposable environments and update covered behavior.
- [x] Run make format, make lint, and make test.
- [ ] Archive completed specifications, merge reviewable PRs, and verify their merged status.


## 6. Classification delivery
- [x] Implement and verify taxonomy lifecycle metadata and retained references on SQLite and PostgreSQL.
- [x] Verify web trash, name reuse, restore conflicts, vault isolation, and viewer restrictions in a disposable browser instance.
- [x] Regenerate OpenAPI and verify native tag compatibility with the Swift package suite.
- [x] Deliver native classification management after its isolated UI verification.

## Vault increment
- [x] Verify both database migrations, scoped metadata, atomic owner creation, retained references, no-op/rollback suppression, and permanent vault/membership audit facts.
- [x] Verify protected defaults, current-owner recovery, viewer/admin isolation, workflow cancellation/fencing, and managed/configured source activity checks.
- [x] Verify usable web owner controls and desktop/mobile lifecycle regression coverage.
- [x] Regenerate API/client contracts and implement Apple metadata, owner management, recovery, and isolated native regressions before completing the full lifecycle change.
- [x] Limit owner lifecycle history to vault and membership facts so document activity cannot push them out of the 100-event window.


## Delivery evidence

The foundation increments are merged:

| Area | Pull requests |
| --- | --- |
| Authenticated actors, vault ownership, event envelopes, and lineage | [#258](https://github.com/cedricziel/folio/pull/258), [#260](https://github.com/cedricziel/folio/pull/260), [#265](https://github.com/cedricziel/folio/pull/265), [#266](https://github.com/cedricziel/folio/pull/266) |
| Classification lifecycle and Apple management | [#279](https://github.com/cedricziel/folio/pull/279), [#315](https://github.com/cedricziel/folio/pull/315) |
| Source/workflow lifecycle, document attribution, and retained source outcomes | [#311](https://github.com/cedricziel/folio/pull/311), [#312](https://github.com/cedricziel/folio/pull/312), [#313](https://github.com/cedricziel/folio/pull/313), [#518](https://github.com/cedricziel/folio/pull/518) |
| Vault lifecycle, owner history filtering, and native authorization | [#317](https://github.com/cedricziel/folio/pull/317), [#503](https://github.com/cedricziel/folio/pull/503), [#510](https://github.com/cedricziel/folio/pull/510), [#512](https://github.com/cedricziel/folio/pull/512) |
| Native source, workflow, and vault management | [#513](https://github.com/cedricziel/folio/pull/513), [#515](https://github.com/cedricziel/folio/pull/515), [#516](https://github.com/cedricziel/folio/pull/516) |

Required server CI ran the workspace Cargo commands behind the Makefile's format,
lint, and test targets, with formatting checked rather than rewritten. Isolated
SQLite/PostgreSQL contracts and web regressions cover authorization, retained
references, name reuse, restore conflicts, and lifecycle history. The source and
workflow attribution tour includes 24 desktop/mobile light/dark captures with no
axe violations, horizontal overflow, or clipped-text findings.

The combined published native stack and OAuth changes passed 760 package tests
and iOS/macOS app builds. Eleven isolated iPhone lifecycle UI cases passed across
the source, workflow, and vault runs, including empty-catalog vault recovery.
Screenshots were reviewed in light appearance; dark appearance, macOS screenshots,
and large Dynamic Type are not claimed by those captures. The package run exposed
an existing Shortcut fixture storage defect; [#569](https://github.com/cedricziel/folio/pull/569)
keeps intent fixtures on in-memory progress storage and explicit test scopes.

The workflow integration item remains open. Document note/comment triggers shipped
in [#522](https://github.com/cedricziel/folio/pull/522), but lifecycle-subject dispatch
for vaults, memberships, taxonomy, ingestion sources, and workflows, plus Apple
trigger selection, belongs to the expanded workflow slice. Keep this change
unarchived until that work and its verification are complete.
