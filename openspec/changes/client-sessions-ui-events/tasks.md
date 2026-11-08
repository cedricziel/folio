## 1. Web and server (PR to main)

- [ ] 1.1 Extend `relay_preserves_otlp_body_and_uses_only_server_credentials` in `crates/folio-api/tests/client_telemetry.rs` with spans and logs carrying `session.id`, `app.screen.name` and `folio.action.name`. It passes against the current relay and stays as the regression guard.
- [ ] 1.2 Add `"test": "node --test"` to `web/package.json`. Write failing tests in `web/telemetry-session.test.mjs` for a new side-effect-free `web/telemetry-session.mjs`, with an injected clock and a fake `Storage`:
  - reuse within 30 min
  - timeout and 4 h rotation, both with `previous_id`
  - a clock set back expiring the session
  - deletion on the sign-in page
  - the in-memory fallback when storage throws
  - rotation inside `navigator.locks` when it exists

  Then implement the module.

- [ ] 1.3 Write failing tests for the vocabulary helpers in the same module:
  - a listed screen passes and an unknown one becomes `other`
  - a repeated screen within 1 s is dropped
  - the 60-per-minute limit and its drop count
  - an unlisted `data-folio-action` emits nothing

  Then implement them.

- [ ] 1.4 Wire `telemetry.mjs`:
  - a span processor that stamps `session.id` and `app.screen.name`
  - `safeSpan` keeping both attributes
  - a log sanitizer with an allowlist
  - `folio.screen.view` on `turbo:load`
  - the delegated action listener
  - log events set by `eventName`
  - a flush on `pagehide`, and after actions on `data-turbo="false"` forms
  - `auth.sign_out` emitting `session.end` before the logout POST

  Rebuild `static/vendor/telemetry.js` with `make assets`.

- [ ] 1.5 Write failing page tests that `/` renders `folio-screen`=`inbox`, `/?view=all` renders `documents`, `/documents/{id}` renders `document.detail` and `/trash` renders `trash`. Add `{% block screen %}` to `base.html` and the values to the full-page templates in the design table.
- [ ] 1.6 Add `data-folio-action` to the web forms and buttons in the action table, including the delete-confirm and logout forms.
- [ ] 1.7 Write failing tests in `crates/folio-api` for:
  - `folio.browse.scope` and `folio.browse.filters` on the `GET /`, `/documents` and `/documents/search` spans (in-memory exporter), with no filter value in any attribute
  - the `Server-Timing` traceparent on HTML responses, and its absence on `/api/` responses

  Implement both in `telemetry.rs` and `routes/pages.rs`.

- [ ] 1.8 Extend `e2e/tests/telemetry.spec.ts` (failing first). The journey: the inbox, a document's detail page, back, then trash, including a delete. Assert that:
  - every exported span and log carries one `session.id`
  - the `folio.screen.view` events read `inbox`, `document.detail`, `inbox`, `trash`
  - a `folio.action` for `document.delete` arrives despite the page unload
  - the first `folio.navigation` has a parent span ID from `Server-Timing`
  - no exported attribute value contains a document UUID, the document title or a query string
- [ ] 1.9 Update `docs/observability.md`: the vocabulary, the lifetime rules, the `Server-Timing` link, server browse attributes, and the three SignalDB queries.
- [ ] 1.10 Run `make format`, `make lint`, `make test`, `cd web && npm test`, `make e2e`, and the `verify` skill against an ephemeral instance.

## 2. Apple (stacked on #377)

- [ ] 2.1 Write failing `TelemetrySessionTests` with an injected clock and a temporary directory. Cover:
  - reuse within 30 min
  - `timeout` and `max_duration` rotation with `previous_id`
  - a clock set back
  - sign-out and server switch deleting the record, with no `previous_id` afterwards
  - an epoch change refusing adoption
  - non-extending reads (File Provider) leaving `lastActivityAt` alone and never starting a session

  Implement `TelemetrySession` (a JSON record plus a `flock` lock file in the App Group container).

- [ ] 2.2 Write failing sharing tests with two `TelemetrySession` instances on the same directory:
  - the second adopts the first's ID
  - a rotation by one is seen by the other on its next read
  - many concurrent rotations converge on one stored ID with exactly one `session.start`

  If the macOS test host allows it, add a `posix_spawn` child-process variant.

- [ ] 2.3 Write failing `TelemetryTests`:
  - a screen and the session start recorded before configuration are exported once the runtime is accepted
  - a same-server reconfigure exports the `session.end`/`session.start` pair it raised, using `retire()`
  - operation spans, URLSession spans and completion logs carry `session.id` and `app.screen.name`
  - metric points carry only `folio.operation` and `folio.outcome`
  - UI events create no span or metric
  - `folio.telemetry.configured` replaces `app.session.start`

  Move session and screen state out of `TelemetryRuntime` and add `uiEvent`.

- [ ] 2.4 Write failing tests for:
  - `TelemetryScreen` and `TelemetryAction` raw values against the vocabulary tables
  - the 1 s screen dedupe
  - the 60-per-minute limit with its drop counter
  - sign-out flushing `session.end` before the transport is disabled

  Add `screen(_:)`, `action(_:)`, the `.telemetryScreen(_:)` modifier and the sign-out flush.

- [ ] 2.5 Write failing `LiveFolioServiceTests` and `DocumentListViewModelTests` cases:
  - `documents.browse` spans carry `folio.browse.scope`, `filters`, `sort` and `page` for Inbox, All documents, a saved view and an intent
  - the browse request's query items are unchanged

  Add the telemetry-only browse context to `DocumentBrowseQuery`.

- [ ] 2.6 Write a failing intents test that each App Intent emits `intent.run` with its `folio.intent.name`, then add the calls in `Intents.swift`.
- [ ] 2.7 Wire activity:
  - the app's `active` phase
  - the share extension opening (`share` screen, `share.submit`/`share.cancel`) in `ShareView`/`ShareViewController`
  - the File Provider as a non-extending reader
  - sign-out and server switch ending the session

  Don't touch `ShareViewModel`, `BackgroundUploader` or the Spotlight code.

- [ ] 2.8 Add one `.telemetryScreen(_:)` line per view in the screen table, and `action(_:)` calls at the listed buttons. `DocumentDetailView*` gets the screen tag only.
- [ ] 2.9 Run `make format` and `make lint`. Apple CI (`test`, `Classification UI`, `Thumbnail UI`) is the Swift gate. After #377 merges, rebase onto main and retarget.

## 3. Close-out

- [ ] 3.1 Check a real session in SignalDB `homelab`/`apps` with the documented queries, and answer the last check of #209 from it.
- [ ] 3.2 Run `make format`, `make lint` and `make test` on main after both PRs merge. Then validate the change, sync the specs and archive it as the last commit.
