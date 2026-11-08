# Design: client sessions and UI events

## Context

- **Apple.** `FolioTelemetry.swift` builds one resource per process (`service.name=folio-app`, `folio.installation.id`, a per-process `service.instance.id`). PR #377 adds `folio.app.component` = `app` | `share` | `fileprovider` and `retire()`, which flushes a replaced runtime before shutting it down. Spans come from `LiveFolioService.call(name)` → `telemetry.operation(name)` and from `URLSessionInstrumentation`. Log records come only from `TelemetryRuntime.completed`, so every Apple span and log is created inside `FolioTelemetry`.
- **Runtime availability.** `FolioTelemetry.current` stays nil until `/api/telemetry/config` answers, and `reserve()` replaces the runtime on every reconfigure: first launch, server change, and a foreground after 5 minutes. Anything that goes through `current` before the runtime is ready is lost.
- **Web.** `web/telemetry.mjs` creates a `folio.navigation` span per Turbo visit. `safeSpan` rebuilds each span from an attribute allowlist and drops span events. Logs go out without a sanitizer. The module starts telemetry on import, and `web/package.json` has no test script.
- **Relay.** `client_telemetry.rs::export` checks auth, content type, size and budget, then posts the body upstream unchanged. It never decodes OTLP, so any attribute a client sets reaches SignalDB as-is.
- **Gap.** `documents.browse` has no scope. On Apple, the Inbox is a client-side filter (`status != filed`) over the same browse call as All documents. On the web, `/` renders the Inbox or every document depending on `?view=`, and the server span records only the route.
- **No link for full page loads.** A full page load has no client HTTP span, so the browser's `folio.navigation` span and the server's `GET /` span end up in unrelated traces.

## Standards checked

Semantic conventions v1.45.0, all **Development** status:

- [Session](https://opentelemetry.io/docs/specs/semconv/general/session/): `session.id` on spans and log records, `session.previous_id`, and `session.start`/`session.end` as named log events. It goes on signals, not the resource, because a resource is fixed for the SDK's life while a session rotates inside one process and spans several processes ([spec#2500](https://github.com/open-telemetry/opentelemetry-specification/issues/2500)).
- [App attributes](https://opentelemetry.io/docs/specs/semconv/registry/attributes/app/): `app.screen.name`, `app.screen.id`, `app.widget.*`. The [app events](https://opentelemetry.io/docs/specs/semconv/app/app-events/) `app.screen.click` and `app.widget.click` require tap coordinates or widget IDs.
- **No screen-view event in semconv.** The experimental [opentelemetry-browser](https://github.com/open-telemetry/opentelemetry-browser/blob/main/docs/browser-observability-model.md) model has `browser.navigation` and `browser.user_action.click`. Its shipped click instrumentation records CSS selectors, element attributes and page coordinates.
- **Mobile lifecycle.** The only lifecycle event is [`device.app.lifecycle`](https://opentelemetry.io/docs/specs/semconv/mobile/mobile-events/) with `ios.app.state`.

What follows: use `session.*` and `app.screen.name` as defined. Screen views and actions get Folio-namespaced events, because the standard click events need coordinates or IDs. Code comments pin the semconv version, since all of this can still change.

## Decision 1: hand-roll, don't adopt a RUM library

| Option                                     | Session rules                                                                                               | Events                                        | Footprint                           | Privacy fit                                         |
| ------------------------------------------ | ----------------------------------------------------------------------------------------------------------- | --------------------------------------------- | ----------------------------------- | --------------------------------------------------- |
| **Web: `@opentelemetry/web-common` 0.301** | timeouts off by default; in-memory timers, so a session restored after a reload isn't checked for staleness | no `session.start`/`end`, no `previous_id`    | 163 KB unpacked; session code ~6 KB | ok, but still needs our events                      |
| **Web: Grafana Faro 2.13**                 | 15 min idle / 4 h max                                                                                       | own names (`session_start`, `session_extend`) | 1.6 MB unpacked                     | collects URLs, errors and console output by default |
| **Web: Honeycomb web 1.5**                 | new ID per page load                                                                                        | none                                          | 1.5 MB, ~20 OTel deps               | only needs a session hook                           |
| **Web: Splunk RUM 3.2 / Elastic RUM 5.17** | 15 min / 4 h; 30 min                                                                                        | none                                          | 7.7 MB / 2 MB                       | vendor attributes; Elastic isn't OTel               |
| **Apple: opentelemetry-swift `Sessions`**  | 30 min idle, no max by default                                                                              | `session.start`/`end` with `previous_id`      | API + SDK only                      | good, but see below                                 |
| **Apple: Embrace 6.x**                     | follows foreground/background                                                                               | session is a parent span                      | full vendor SDK                     | too broad                                           |
| **Hand-rolled, both platforms**            | ours                                                                                                        | ours, matching semconv                        | ~150 lines each, no new deps        | fixed vocabulary by construction                    |

Sources: [web-common](https://open-telemetry.github.io/opentelemetry-js/modules/_opentelemetry_web-common.html), [Faro session tracking](https://grafana.com/docs/grafana-cloud/monitor-applications/frontend-observability/instrument/session-tracking/), [Honeycomb](https://www.honeycomb.io/blog/manage-your-session-id-honeycombs-web-sdk), [Splunk RUM config](https://help.splunk.com/splunk-observability-cloud/manage-data/instrument-front-end-applications/instrument-mobile-and-web-applications-for-splunk-rum/instrument-browser-applications-for-splunk-rum/configure-the-splunk-rum-browser-agent), [opentelemetry-swift Sessions](https://github.com/open-telemetry/opentelemetry-swift/tree/main/Sources/Instrumentation/Sessions), [Embrace sessions](https://embrace.io/docs/ios/6x/core-concepts/sessions/).

**Recommendation: hand-roll on both platforms.**

- **Web.** The libraries are either bigger than the rest of the telemetry bundle or still need the parts we care about (events, `previous_id`, staleness checks on load). The useful piece of web-common, a span processor that stamps an attribute, is a few lines.
- **Apple.** The `Sessions` product is the closest fit, but its `SessionManager` reads persistence once, at init, and then works on an in-memory copy. A File Provider extension can live for hours, so it would never see the app rotate the session on sign-out or start a new one. Sharing through the App Group needs a read-through store under a cross-process lock. FolioKit doesn't use that product today, so adopting it would also tie us to an instrumentation API that is still changing. A ~150-line store with an injected clock is easier to test against our exact rules.

## Decision 2: session lifetime

| Rule                  | Value                                                                                                                                                                                       | Why                                                                                                                                                                                 |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Inactivity timeout    | 30 min                                                                                                                                                                                      | Matches opentelemetry-swift and Elastic. Faro and Splunk use 15 min; 30 keeps a document read in a background tab, or a pause to find a paper, in one journey.                      |
| Maximum length        | 4 h                                                                                                                                                                                         | Faro and Splunk. Bounds a session left open all day.                                                                                                                                |
| ID                    | random UUID v4, lowercase                                                                                                                                                                   | `crypto.randomUUID` / `UUID()`. Not derived from anything.                                                                                                                          |
| Activity              | a screen view, an action, the app becoming active, the share extension opening                                                                                                              | These are the only things that extend a session or may rotate an expired one.                                                                                                       |
| Not activity          | network calls, background uploads (`BackgroundUploader`), Spotlight indexing, File Provider callbacks, Turbo frame and SSE fetches, telemetry reconfiguration                               | These never extend, rotate or start a session. When they run with no live session, their records carry no `session.id`.                                                             |
| Timeout or max length | the next activity ends the old session (`session.end`, reason `timeout` or `max_duration`, timestamped at its last activity) and starts a new one whose `session.previous_id` is the old ID | Staleness is checked when the session is read, never with timers, so reloads, suspended extensions and sleeping laptops behave the same. Both reasons link to the previous session. |
| Clock set back        | if `now` is earlier than `lastActivityAt` or `startedAt`, the session counts as expired with reason `timeout`                                                                               | Otherwise a clock set back by an hour would freeze both the timeout and the 4 h cap.                                                                                                |
| Sign-out              | best effort: emit `session.end` with reason `sign_out` and flush while the transport still has credentials. Then delete the record and bump the sign-in epoch.                              | The next sign-in has no `previous_id`. The installation ID still ties the two accounts to the same device, so this only avoids adding a second link.                                |
| Server switch         | same as sign-out, with reason `server_change`                                                                                                                                               |                                                                                                                                                                                     |

**Sign-in epoch.** The shared record carries a random `epoch` written at sign-in. Each surface remembers the epoch it last saw. If a surface finds a different epoch, or no record, it never adopts a session from before. An extension that was running across a sign-out and sign-in then can't carry the old session into the new account.

**Apple storage.** One JSON file in the App Group container, `telemetry-session.json`: `{epoch, id, previousID, startedAt, lastActivityAt}`. Each read-modify-write happens under an exclusive `flock` on `telemetry-session.lock` in the same container, the same pattern `OAuthRefreshLock` uses. The lock is held only for the read, the decision and the write, which takes microseconds, so taking it synchronously on the stamping path is fine. Under the lock, two processes that both find an expired session can't both rotate it: the second reads the first's new record and adopts it.

**Web storage.** One `localStorage` record, key `folio.telemetry.session`, with the same fields. Rotation runs inside `navigator.locks.request("folio.telemetry.session", …)` so tabs can't race. Where the Web Locks API is missing, the store falls back to read-check-write and accepts the rare extra session, linked by `previous_id`. If storage is blocked, the session lives in memory for that page, the same fallback the installation ID uses. `navigator.locks` is async, so stamping reads the in-memory copy, which is refreshed on each activity and on the `storage` event.

## Decision 3: which Apple surfaces use the app's session

| Option                                               | Effect                                                                                                                                                                                           |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Join the live session (chosen)**                   | Share sheet → open Folio → check the upload is one session. The breadcrumb trail follows the user, which is the goal. `folio.app.component` (#377) still says which process emitted each record. |
| Own session per extension, linked with `previous_id` | Datadog's model (`session.is_main_process`). Every share or Files visit becomes a separate session, and following a journey needs a join.                                                        |

Rules per surface:

- **App and App Intents.** They count as activity: they extend the live session or rotate an expired one. Intents run in the app process today. If one moves to an extension, the same rule applies.
- **Share extension.** It counts as activity too: the person opened the share sheet deliberately.
- **File Provider.** It reads the record and stamps a live session it finds, but never extends, rotates or starts one. With no live session, its records carry no `session.id`. Files traffic made while the app or share sheet was recently in use is still grouped with that journey.

## Decision 4: keep sessions and screens outside the export runtime (Apple)

In `FolioTelemetry`, session state and the current screen belong to the process, not to the runtime that is rebuilt on every reconfigure.

- **Process state.** `FolioTelemetry` holds a `TelemetrySession` (the shared store above) and `currentScreen`. Both are updated even while no runtime exists.
- **Queue until ready.** Session boundary events (`session.end`, `session.start`) raised while there is no runtime go into a small queue in `FolioTelemetry` (at most 8, oldest dropped). When a runtime is accepted, the queue is emitted first, then the current screen is re-emitted as `folio.screen.view`. This covers a cold launch on the Inbox and a session that rotates on the 5-minute foreground reconfigure.
- **Actions before ready.** Actions raised before the runtime is ready are dropped. A queue of user actions with no export target is the kind of buffering the privacy contract avoids, and the screen and session context still arrive.
- **Flush before replacing.** On a same-server reconfigure, the old runtime is flushed (`retire()` from #377) before shutdown, so the `session.end`/`session.start` pair emitted on a foreground isn't discarded. A server change, sign-out or disabled configuration still discards, after the best-effort sign-out flush in Decision 2.
- **Separate UI event path.** `TelemetryRuntime` gains `uiEvent(name:attributes:)`. It emits one log record and nothing else: no span and no metric. `event()` is kept only for the renamed `folio.telemetry.configured`.
- **No high-cardinality metric attributes.** `completed()` builds its metric attributes and log attributes separately. `session.id`, `app.screen.name` and the browse context go on spans and logs only. Metric attributes stay `folio.operation` and `folio.outcome`.

## Decision 5: screens

- **Event.** `folio.screen.view` log event with `app.screen.name`, emitted when the visible screen changes. A repeat of the same screen within one second is dropped (SwiftUI `onAppear` fires again on pops, sheet dismissals and tab switches).
- **Current screen.** The last screen name is also stamped as `app.screen.name` on every later span and log record, so a `documents.browse` or `GET /api/...` span says where it was issued.
- **Apple tagging.** A `.telemetryScreen(_ screen: TelemetryScreen)` view modifier in FolioKit (`onAppear` → `FolioTelemetry.shared.screen(screen)`). It takes an enum, so a string can't slip in. One line per view; path observation was rejected because it doesn't see sheets, and most screens here are sheets or split-view columns.
- **Web tagging.** `base.html` renders `<meta name="folio-screen" content="{% block screen %}other{% endblock %}">`, and each full-page template overrides the block. `telemetry.mjs` reads it on `turbo:load` and checks it against the same allowlist. Mapping URL routes in JavaScript was rejected: it would duplicate the router. Partials rendered into Turbo frames (search results, document list frames) aren't screens and emit nothing.

## Decision 6: actions

- **Event.** `folio.action` log event with `folio.action.name` from the vocabulary below, plus a few fixed-value qualifiers. No outcome on the action: the operation span that follows has one.
- **Apple.** `FolioTelemetry.shared.action(.documentDelete)` at the call site of each button or intent.
- **Web.** `data-folio-action="document.delete"` on a form or button. One delegated `submit` and `click` listener reads the attribute; anything outside the allowlist is ignored. Nothing is read from the DOM except that attribute and the screen meta tag.
- **Web unload.** Several forms are `data-turbo="false"` (delete confirm, logout) and unload the page right after submitting. On `pagehide`, and immediately after an action on such a form, the log and trace processors flush. The OTLP exporters send with `keepalive: true`, so the request survives the unload. This is best effort. A lost action is acceptable, but the e2e test checks that the common case arrives.
- **Web sign-out.** The logout form carries `data-folio-action="auth.sign_out"`. The listener emits the action, then `session.end` (reason `sign_out`), and flushes with `keepalive` before the POST clears relay authentication. The sign-in page then deletes the record, whether or not the export arrived.

## Decision 7: session ID stays on the client; trace context joins the server

- **No baggage.** `session.id` isn't sent to the server as W3C baggage or any other header. Baggage would put the ID into server logs and any downstream hop, and needs server code to copy and validate it. No OTel convention asks for it, and Honeycomb, Splunk and Faro don't send it by default. `TelemetryHTTPClient` already strips `baggage` from export requests; keep that.
- **Client HTTP spans.** These already propagate `traceparent` to the Folio origin, so a session's server spans are one join away: session → client trace IDs → server spans with the same trace ID.
- **Full page loads.** The server adds `Server-Timing: traceparent;desc="00-<trace>-<span>-01"` to HTML responses, from the request span in `folio-api/src/telemetry.rs`. On the first `folio.navigation` of a page load, `telemetry.mjs` reads `performance.getEntriesByType("navigation")[0].serverTiming` and starts its span as a child of that context. Turbo visits are fetches and already carry `traceparent`. Splunk RUM uses the same header for the same reason.
  - **Why a header rather than the meta tag that was asked for.** Askama templates share no context, so a meta tag would need a field on every page struct. The header is one line in the existing middleware. The values are trace and span IDs only, and the header is sent only on same-origin HTML responses.

## Decision 8: server records web browse scope

The web list is server-rendered, so the server span is where its browse context exists. The `GET /` and `GET /documents` request spans, and `GET /documents/search`, record:

- `folio.browse.scope`: `inbox` when no `view` is set, `all` for `view=all`, `search` for the search route
- `folio.browse.filters`: a sorted array of the filter kinds present, from `FilterQuery` (`reviewed`, `document_date`, `correspondent`, `document_type`, `tag`), plus `text` for search. Never their values.

These go on the server span only, never on `http.server.request.duration` or any other metric.

## Decision 9: relay

The relay already forwards the body unchanged. Keep it that way: decoding OTLP to filter attributes would duplicate the client allowlists, cost CPU per request and make the relay protocol-aware. Extend `relay_preserves_otlp_body_and_uses_only_server_credentials` in `crates/folio-api/tests/client_telemetry.rs` with a body that carries `session.id`, `app.screen.name` and `folio.action.name`, and say so in the observability spec and docs.

## Vocabulary

### Attributes

| Attribute                  | Where                                                                                                     | Values                                                                                                                                                     |
| -------------------------- | --------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `session.id`               | every client span and log record while a session is live                                                  | UUID v4, lowercase                                                                                                                                         |
| `session.previous_id`      | `session.start` only, when it follows a `timeout` or `max_duration` session                               | UUID v4                                                                                                                                                    |
| `folio.session.end_reason` | `session.end`                                                                                             | `timeout`, `max_duration`, `sign_out`, `server_change`                                                                                                     |
| `app.screen.name`          | `folio.screen.view`; every later client span and log record                                               | screen vocabulary                                                                                                                                          |
| `folio.action.name`        | `folio.action`                                                                                            | action vocabulary                                                                                                                                          |
| `folio.upload.source`      | `folio.action` for `document.upload`                                                                      | `picker`, `scan`, `drop`, `paste`, `share`, `intent`                                                                                                       |
| `folio.intent.name`        | `folio.action` for `intent.run`                                                                           | `scan_document`, `upload_file`, `search_documents`, `find_documents`, `get_latest_document`, `open_document`, `add_tag`, `remove_tag`, `get_document_file` |
| `folio.browse.scope`       | Apple `documents.browse` span and completion log; server `GET /`, `/documents`, `/documents/search` spans | Apple: `inbox`, `all`, `saved_view`, `intent`. Server: `inbox`, `all`, `search`                                                                            |
| `folio.browse.filters`     | same                                                                                                      | string array, a sorted subset of `text`, `tag`, `correspondent`, `document_type`, `document_date`, `created_date`, `reviewed`; empty when none             |
| `folio.browse.sort`        | Apple only                                                                                                | `newest`, `oldest`, `title`                                                                                                                                |
| `folio.browse.page`        | Apple only                                                                                                | `first`, `next`                                                                                                                                            |

`folio.browse.filters` records which kinds of filter are set, never their values.

### Events (log records with an event name)

Each event sets the log record's event name (`eventName` in JS, `setEventName` in Swift). The body is left empty.

| Event               | Attributes                                                             |
| ------------------- | ---------------------------------------------------------------------- |
| `session.start`     | `session.id`, `session.previous_id`?                                   |
| `session.end`       | `session.id`, `folio.session.end_reason`                               |
| `folio.screen.view` | `session.id`, `app.screen.name`                                        |
| `folio.action`      | `session.id`, `app.screen.name`, `folio.action.name`, qualifiers above |

The Apple `app.session.start` event is renamed `folio.telemetry.configured`, because it fires each time telemetry is configured, not once per session. The `app.lifecycle.*` events stay.

### Screens

| Name                  | Apple                                    | Web template                                                    |
| --------------------- | ---------------------------------------- | --------------------------------------------------------------- |
| `onboarding`          | OnboardingView                           | —                                                               |
| `app_lock`            | AppLockView                              | —                                                               |
| `inbox`               | InboxView with Inbox destination         | `index.html` without `view`                                     |
| `documents`           | InboxView with All documents destination | `index.html` with `view=all`                                    |
| `saved_view`          | saved view opened in the browser         | `saved_view.html` showing a view                                |
| `saved_views`         | —                                        | `saved_views.html`                                              |
| `saved_view.editor`   | SavedViewEditor                          | `saved_view.html` in new or edit mode                           |
| `document.detail`     | DocumentDetailView (tag only)            | `document_view.html` (`/documents/{id}`)                        |
| `document.preview`    | QuickLookPreview                         | —                                                               |
| `document.discussion` | DocumentDiscussionView                   | — (part of the detail page)                                     |
| `document.processing` | — (part of the detail view)              | —                                                               |
| `scan.review`         | ScanReviewView                           | —                                                               |
| `upload.review`       | UploadReviewView                         | —                                                               |
| `taxonomy`            | TaxonomyView                             | `manage.html`                                                   |
| `sources`             | SourceActivityView                       | `sources.html`, `source_form.html`                              |
| `source.outcomes`     | SourceOutcomesView                       | `source_outcomes.html`                                          |
| `trash`               | —                                        | `trash.html`                                                    |
| `workflows`           | —                                        | `workflows.html`, `workflow_form.html`, `workflow_history.html` |
| `integrations`        | —                                        | `integrations.html`                                             |
| `settings`            | —                                        | `settings.html`                                                 |
| `settings.security`   | —                                        | `settings_security.html`                                        |
| `settings.vaults`     | —                                        | `vaults.html`                                                   |
| `share`               | ShareView                                | —                                                               |
| `error`               | —                                        | `error.html`                                                    |
| `other`               | fallback                                 | fallback                                                        |

- **No web screen.** Search results (`partials/search_results.html`) and the other partials are Turbo frames, not screens. A search shows up as a `search.submit` action and a `/documents/search` server span with scope `search`. `document_detail.html` isn't rendered by any route and gets no tag.
- **Untracked pages.** Sign-in, setup, consent and other `auth/*` pages stay untracked, as today.

### Actions

| Name                                                      | Apple                                      | Web                      |
| --------------------------------------------------------- | ------------------------------------------ | ------------------------ |
| `document.open`                                           | row tap                                    | row link                 |
| `document.upload`                                         | picker, scan, drop, paste, share, intent   | upload form, drop, paste |
| `document.delete`                                         | ✓                                          | ✓                        |
| `document.restore`                                        | ✓                                          | trash restore            |
| `document.purge`                                          | —                                          | trash delete forever     |
| `document.review`                                         | mark reviewed / unreviewed                 | ✓                        |
| `document.download`                                       | ✓                                          | ✓                        |
| `document.share`                                          | share sheet from detail                    | —                        |
| `document.reprocess`                                      | ✓                                          | ✓                        |
| `document.edit`                                           | title, date, type, correspondent           | same forms               |
| `document.tag.add`, `document.tag.remove`                 | ✓                                          | ✓                        |
| `discussion.post`                                         | ✓                                          | ✓                        |
| `browse.scope.change`                                     | Inbox ↔ All documents                      | —                        |
| `browse.filter.change`, `browse.sort.change`              | ✓                                          | ✓                        |
| `browse.load_more`                                        | ✓                                          | —                        |
| `browse.refresh`                                          | pull to refresh                            | —                        |
| `search.submit`                                           | ✓                                          | ✓                        |
| `saved_view.open`, `saved_view.save`, `saved_view.delete` | ✓                                          | ✓                        |
| `vault.switch`                                            | ✓                                          | vault picker             |
| `scan.start`, `scan.finish`, `scan.cancel`                | ✓                                          | —                        |
| `share.submit`, `share.cancel`                            | share extension                            | —                        |
| `auth.sign_out`                                           | ✓                                          | logout form              |
| `app_lock.unlock`                                         | ✓                                          | —                        |
| `intent.run`                                              | every App Intent, with `folio.intent.name` | —                        |

## Cost and cardinality in SignalDB

- **High-cardinality values stay off metrics.** `session.id` is never a metric attribute or a resource attribute. SignalDB stores span and log attributes per record, the same as trace IDs, so a new UUID per session adds no series. Metrics keep their current attribute sets; screen names, action names and browse context aren't added to any metric.
- **Fixed vocabularies.** ~25 screens, ~35 actions and four browse attributes with bounded values.
- **Volume.** One log record per screen change and per action, plus two per session. A busy session (100 screen changes, 50 actions) adds ~150 small log records, below the span volume of its network calls.
- **Limits.** Consecutive identical screen views within 1 s are dropped. Each process allows 60 UI events (screens plus actions) per minute and drops the rest, counted in `folio.telemetry.ui_events.dropped` (no attributes). The relay's 120 requests per minute per user and the batch processors' queue limits still apply.
- **Sampling.** None: a single-household deployment produces too little for it to matter. If it's ever needed, sample whole sessions by hashing `session.id` on the client, so a session is kept or dropped as one.

## Following a session

`docs/observability.md` will show these queries against `homelab`/`apps`:

1. Find recent sessions: `session.start` log events by `service.name` and `folio.app.component`.
2. Breadcrumbs: logs where `session.id = X`, ordered by time (`folio.screen.view`, `folio.action`, `folio.operation.completed`).
3. Server side: trace IDs of spans with `session.id = X`, then server spans in those traces.

## Testing

- **Web session store.** The store and the vocabulary checks move into a pure module, `web/telemetry-session.mjs`, with no side effects on import. `telemetry.mjs` imports it. `web/package.json` gains `"test": "node --test"`, and the tests use an injected clock and a fake `Storage`.
- **Apple cross-process.** Two `TelemetrySession` instances over separate `FileManager` paths in one temporary directory exercise the file-and-lock protocol, since that is what two processes share. A concurrency test runs rotations from many tasks against two instances and checks they converge on one stored ID. A helper process can't be spawned from an iOS test bundle. If CI's macOS test host allows it, an extra macOS-only test forks a child with `posix_spawn` and runs the same check.

## Risks

- **Semconv may change.** `session.*` and `app.screen.name` are Development. The names live in one constant table per platform, so a rename is one edit.
- **Web log sanitizing is new.** Today logs aren't filtered. Adding an allowlist could drop the existing `folio.web.navigation` attributes if it's wrong; the e2e test asserts them.
- **Sign-out events are best effort.** The flush can lose the race with the logout POST or process exit. Deleting the record is the guarantee; the `session.end` event is a courtesy.
- **Rebases.** The Apple tag additions touch many views, so they are one line each and land last in the Apple PR.

## Decided questions

1. **Inactivity timeout:** 30 min.
2. **File Provider:** it joins a live session without extending it, and never starts one.
3. **Web browse scope:** the server records it on `/`, `/documents` and `/documents/search` request spans (Decision 8). Full page loads join their server request through `Server-Timing` (Decision 7).
4. **`app.session.start`:** renamed to `folio.telemetry.configured`.
