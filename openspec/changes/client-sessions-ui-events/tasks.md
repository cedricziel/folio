## 1. Web and relay (PR to main)

- [ ] 1.1 Write failing `relay_forwards_client_attributes_unchanged` in `crates/folio-api` (posts OTLP JSON with `session.id`, `app.screen.name`, `folio.action.name` to a stub upstream and compares bytes). It is expected to pass against the current relay; keep it as the regression guard.
- [ ] 1.2 Write failing unit tests for the web session store (`web/telemetry.test.mjs`, `node --test`): reuse within 30 min, rotation after 30 min with `previous_id`, rotation at 4 h, deletion on the sign-in page, in-memory fallback when `localStorage` throws. Implement the store in `web/telemetry.mjs` with an injected clock.
- [ ] 1.3 Write a failing unit test that `safeSpan` keeps `session.id` and `app.screen.name` and that the new log sanitizer drops attributes outside the allowlist. Add the span processor that stamps them and the log sanitizer.
- [ ] 1.4 Write a failing unit test for screen-name validation (vocabulary passes, unknown becomes `other`, a repeat within 1 s is dropped) and the 60-per-minute UI event limit. Add `folio.screen.view` on `turbo:load` from `<meta name="folio-screen">`.
- [ ] 1.5 Add `{% block screen %}` to `base.html` and the screen value to each page template in the design table. Write a failing Rust page test that `/` renders `folio-screen` = `documents` and `/trash` renders `trash`.
- [ ] 1.6 Write a failing unit test for the delegated action listener (listed `data-folio-action` emits, unlisted emits nothing, no other DOM data read). Add `data-folio-action` to the web forms and buttons in the action table.
- [ ] 1.7 Extend `e2e/tests/telemetry.spec.ts` (failing first): after a document list → detail → back → trash journey, exported spans and logs carry one `session.id`, `folio.screen.view` events list `documents`, `document.detail`, `documents`, `trash`, a `folio.action` for `document.delete` exists, and no exported attribute value contains a document UUID, the document title or a query string.
- [ ] 1.8 Update `docs/observability.md`: vocabulary, lifetime rules, the note that `app.session.start` is not the session start, and the three SignalDB queries.
- [ ] 1.9 Rebuild `static/vendor/telemetry.js` with `make assets`. Run `make format`, `make lint`, `make test`, `make e2e`, and the `verify` skill against an ephemeral instance.

## 2. Apple (stacked on #377)

- [ ] 2.1 Write failing `TelemetrySessionTests` with an injected clock and an in-memory store: reuse within 30 min; timeout rotation emits `session.end` (`timeout`) then `session.start` with `previous_id`; 4 h rotation; sign-out and server switch delete the record with no `previous_id` afterwards; non-extending reads (File Provider) don't move `lastActivityAt`. Implement `TelemetrySession` in `FolioKit/Telemetry`.
- [ ] 2.2 Write failing App Group sharing tests with two `TelemetrySession` instances on one `UserDefaults` suite: the second adopts the first's ID; a rotation by one is seen by the other on its next read; concurrent starts converge on the stored record.
- [ ] 2.3 Write failing `TelemetryTests` cases: operation spans, URLSession spans and completion logs carry `session.id` and the current `app.screen.name`; the resource keys are unchanged. Stamp them in `FolioTelemetry`.
- [ ] 2.4 Write failing tests for `TelemetryScreen` and `TelemetryAction` raw values against the vocabulary tables, the 1 s screen dedupe and the 60-per-minute limit with its drop counter. Add `screen(_:)`, `action(_:)` and the `.telemetryScreen(_:)` modifier.
- [ ] 2.5 Write failing `LiveFolioServiceTests` and `DocumentListViewModelTests` cases: `documents.browse` spans carry `folio.browse.scope`, `filters`, `sort` and `page` for Inbox, All documents, a saved view and an intent, and the browse request's query items are unchanged. Add the telemetry-only browse context to `DocumentBrowseQuery`.
- [ ] 2.6 Write a failing intents test that each App Intent emits `intent.run` with its `folio.intent.name`. Add the calls in `Intents.swift`.
- [ ] 2.7 Wire activity: app `active` phase, share extension start (`share` screen, `share.submit`/`share.cancel`), File Provider as a non-extending reader, sign-out and server switch ending the session. Keep `ShareViewModel`, `BackgroundUploader` and Spotlight code untouched; the share hooks go in `ShareView`/`ShareViewController`.
- [ ] 2.8 Add one `.telemetryScreen(_:)` line per view in the screen table and `action(_:)` calls at the listed buttons. `DocumentDetailView*` gets the screen tag only.
- [ ] 2.9 Run `make format` and `make lint`; Apple CI (`test`, `Classification UI`, `Thumbnail UI`) is the Swift gate. After #377 merges, rebase onto main and retarget.

## 3. Close-out

- [ ] 3.1 Check a real session in SignalDB `homelab`/`apps` with the documented queries and answer the last check of #209 from it.
- [ ] 3.2 Run `make format`, `make lint` and `make test` on main after both PRs merge, then validate the change, sync the specs and archive it as the last commit.
