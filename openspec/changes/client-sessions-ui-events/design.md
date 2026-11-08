# Design: client sessions and UI events

## Context

- **Apple.** `FolioTelemetry.swift` builds one resource per process (`service.name=folio-app`, `folio.installation.id`, a per-process `service.instance.id`). PR #377 adds `folio.app.component` = `app` | `share` | `fileprovider`. Spans come from `LiveFolioService.call(name)` → `telemetry.operation(name)` and from `URLSessionInstrumentation`. Log records come only from `TelemetryRuntime.completed`, so every Apple span and log is created inside `FolioTelemetry`.
- **Web.** `web/telemetry.mjs` creates a `folio.navigation` span per Turbo visit. `safeSpan` rebuilds each span from an attribute allowlist and drops span events. Logs go out without a sanitizer.
- **Relay.** `client_telemetry.rs::export` checks auth, content type, size and budget, then posts the body upstream unchanged. It never decodes OTLP, so any attribute a client sets reaches SignalDB as-is.
- **Gap.** `documents.browse` has no scope. The Inbox is a client-side filter (`status != filed`) over the same browse call as All documents, so the server request can't tell them apart either.

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
- **Apple.** The `Sessions` product is the closest fit, but its `SessionManager` reads persistence once, at init, and then works on an in-memory copy. A File Provider extension can live for hours, so it would never see the app rotate the session on sign-out or start a new one. Sharing through the App Group needs a read-through store. FolioKit doesn't use that product today, so adopting it would also tie us to an instrumentation API that is still changing. A ~150-line store with an injected clock is easier to test against our exact rules.

## Decision 2: session lifetime

| Rule                  | Value                                                                                                                                                                   | Why                                                                                                                                                            |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Inactivity timeout    | 30 min                                                                                                                                                                  | Matches opentelemetry-swift and Elastic. Faro and Splunk use 15 min; 30 keeps a document read in a background tab, or a pause to find a paper, in one journey. |
| Maximum length        | 4 h                                                                                                                                                                     | Faro and Splunk. Bounds a session left open all day.                                                                                                           |
| ID                    | random UUID v4, lowercase                                                                                                                                               | `crypto.randomUUID` / `UUID()`. Not derived from anything.                                                                                                     |
| Activity              | screen view, action, app becoming active                                                                                                                                | Network calls and File Provider enumerations are not activity, so background work can't keep a session alive.                                                  |
| Timeout or max length | the next use ends the old session (`session.end`, reason `timeout` or `max_duration`, timestamped at its last activity) and starts a new one with `session.previous_id` | Staleness is checked when the session is read, never with timers, so reloads, suspended extensions and sleeping laptops behave the same.                       |
| Sign-out              | `session.end` reason `sign_out`; record deleted                                                                                                                         | The next sign-in has no `previous_id`, so two accounts on one device are never linked.                                                                         |
| Server switch         | `session.end` reason `server_change`; record deleted                                                                                                                    | Same as sign-out.                                                                                                                                              |

**Apple storage.** One record in the App Group `UserDefaults` suite (`group.com.cedricziel.folio`), key `folio.telemetry.session`: `{id, previousID, startedAt, lastActivityAt}`. Every read goes to the suite (cfprefsd keeps that cheap), and every write is a whole record. Two processes can race to start a session; the loser re-reads and adopts the winner's ID. The worst case is one extra short session, which `previous_id` links.

**Web storage.** One `localStorage` record, key `folio.telemetry.session`, with the same fields, shared by tabs on the origin. If storage is blocked the session lives in memory for that page, the same fallback the installation ID uses.

## Decision 3: extensions join the app's session

| Option                                               | Effect                                                                                                                                                                                           |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Join the live session (recommended)**              | Share sheet → open Folio → check the upload is one session. The breadcrumb trail follows the user, which is the goal. `folio.app.component` (#377) still says which process emitted each record. |
| Own session per extension, linked with `previous_id` | Datadog's model (`session.is_main_process`). Every share or Files visit becomes a separate session, and following a journey needs a join.                                                        |

Rules per surface:

- **App and App Intents.** They read the shared session, rotating it if it has expired, and count as activity. Intents run in the app process today. If one moves to an extension, the same rule applies.
- **Share extension.** It joins or starts a session and counts as activity: the person opened the share sheet deliberately.
- **File Provider.** It joins a live session but never extends it. If none is live it starts one, so Files traffic is still grouped, and that session times out 30 minutes after it started unless the app or share sheet becomes active. Files browsing is user-driven, but the provider can't tell user-driven enumerations from system ones.

## Decision 4: screens

- **Event.** `folio.screen.view` log event with `app.screen.name`, emitted when the visible screen changes. A repeat of the same screen within one second is dropped (SwiftUI `onAppear` fires again on pops, sheet dismissals and tab switches).
- **Current screen.** The last screen name is also stamped as `app.screen.name` on every later span and log record, so a `documents.browse` or `GET /api/...` span says where it was issued.
- **Apple tagging.** A `.telemetryScreen(_ screen: TelemetryScreen)` view modifier in FolioKit (`onAppear` → `FolioTelemetry.shared.screen(screen)`). It takes an enum, so a string can't slip in. One line per view; path observation was rejected because it doesn't see sheets, and most screens here are sheets or split-view columns.
- **Web tagging.** `base.html` renders `<meta name="folio-screen" content="{% block screen %}other{% endblock %}">`, and each page template overrides the block. `telemetry.mjs` reads it on `turbo:load` and checks it against the same allowlist. Mapping URL routes in JavaScript was rejected: it would duplicate the router and mis-handle `/documents/search` versus `/documents/{id}`.

## Decision 5: actions

- **Event.** `folio.action` log event with `folio.action.name` from the vocabulary below, plus a few fixed-value qualifiers. No outcome on the action: the operation span that follows has one.
- **Apple.** `FolioTelemetry.shared.action(.documentDelete)` at the call site of each button or intent.
- **Web.** `data-folio-action="document.delete"` on a form or button. One delegated `submit` and `click` listener reads the attribute; anything outside the allowlist is ignored. Nothing is read from the DOM except that attribute.

## Decision 6: no session ID to the server

- **Options.** (a) W3C `baggage: session.id=…` to the Folio origin, with the server copying it onto request spans. (b) Nothing new.
- **Recommendation: (b).** Client spans already propagate `traceparent` to the Folio origin, so a session's server spans are one join away: session → client trace IDs → server spans with the same trace ID. Baggage would put the ID into server logs and any downstream hop, and needs server code to copy and validate it. No OTel convention asks for it, and Honeycomb, Splunk and Faro don't send it by default. `TelemetryHTTPClient` already strips `baggage` from export requests; keep that.

## Decision 7: relay

The relay already forwards the body unchanged. Keep it that way: decoding OTLP to filter attributes would duplicate the client allowlists, cost CPU per request and make the relay protocol-aware. Add a regression test that a body carrying `session.id` and `app.screen.name` arrives upstream byte for byte, and say so in the observability spec and docs.

## Vocabulary

### Attributes

| Attribute                  | Where                                                     | Values                                                                                                                                                     |
| -------------------------- | --------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `session.id`               | every client span and log record                          | UUID v4, lowercase                                                                                                                                         |
| `session.previous_id`      | `session.start` only, when it follows a timed-out session | UUID v4                                                                                                                                                    |
| `folio.session.end_reason` | `session.end`                                             | `timeout`, `max_duration`, `sign_out`, `server_change`                                                                                                     |
| `app.screen.name`          | `folio.screen.view`; every later span and log record      | screen vocabulary                                                                                                                                          |
| `folio.action.name`        | `folio.action`                                            | action vocabulary                                                                                                                                          |
| `folio.upload.source`      | `folio.action` for `document.upload`                      | `picker`, `scan`, `drop`, `paste`, `share`, `intent`                                                                                                       |
| `folio.intent.name`        | `folio.action` for `intent.run`                           | `scan_document`, `upload_file`, `search_documents`, `find_documents`, `get_latest_document`, `open_document`, `add_tag`, `remove_tag`, `get_document_file` |
| `folio.browse.scope`       | `documents.browse` span and its completion log            | `inbox`, `all`, `saved_view`, `intent`                                                                                                                     |
| `folio.browse.filters`     | same                                                      | string array, a sorted subset of `text`, `tag`, `correspondent`, `document_type`, `document_date`, `created_date`, `reviewed`; empty when none             |
| `folio.browse.sort`        | same                                                      | `newest`, `oldest`, `title`                                                                                                                                |
| `folio.browse.page`        | same                                                      | `first`, `next`                                                                                                                                            |

`folio.browse.filters` records which kinds of filter are set, never their values.

### Events (log records with an event name)

| Event               | Attributes                                                             |
| ------------------- | ---------------------------------------------------------------------- |
| `session.start`     | `session.id`, `session.previous_id`?                                   |
| `session.end`       | `session.id`, `folio.session.end_reason`                               |
| `folio.screen.view` | `session.id`, `app.screen.name`                                        |
| `folio.action`      | `session.id`, `app.screen.name`, `folio.action.name`, qualifiers above |

The existing `app.session.start` and `app.lifecycle.*` Apple events stay. `app.session.start` means "telemetry configured for this process"; the docs will say it is not the session start.

### Screens

| Name                  | Apple                                    | Web template                                                    |
| --------------------- | ---------------------------------------- | --------------------------------------------------------------- |
| `onboarding`          | OnboardingView                           | —                                                               |
| `app_lock`            | AppLockView                              | —                                                               |
| `inbox`               | InboxView with Inbox destination         | —                                                               |
| `documents`           | InboxView with All documents destination | `index.html`                                                    |
| `saved_view`          | saved view opened in the browser         | `saved_view.html`                                               |
| `saved_views`         | —                                        | `saved_views.html`                                              |
| `saved_view.editor`   | SavedViewEditor                          | `saved_view.html` in new/edit mode                              |
| `search`              | —                                        | search results page                                             |
| `document.detail`     | DocumentDetailView (tag only)            | `document_detail.html`                                          |
| `document.preview`    | QuickLookPreview                         | `document_view.html`                                            |
| `document.discussion` | DocumentDiscussionView                   | — (part of detail)                                              |
| `document.processing` | DocumentProcessingView                   | —                                                               |
| `scan.review`         | ScanReviewView                           | —                                                               |
| `upload.review`       | UploadReviewView                         | —                                                               |
| `taxonomy`            | TaxonomyView                             | `manage.html` and `/manage/*` lists                             |
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

Sign-in and setup pages stay untracked, as today.

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

- **High-cardinality values stay off metrics.** `session.id` is never a metric attribute or a resource attribute. SignalDB stores span and log attributes per record, the same as trace IDs, so a new UUID per session adds no series. Metrics keep their current attribute sets; screen and action names aren't added to metrics in this change.
- **Fixed vocabularies.** ~25 screens, ~35 actions and four browse attributes with bounded values.
- **Volume.** One log record per screen change and per action, plus two per session. A busy session (100 screen changes, 50 actions) adds ~150 small log records, below the span volume of its network calls.
- **Limits.** Consecutive identical screen views within 1 s are dropped. Each process allows 60 UI events (screens plus actions) per minute and drops the rest, counted in `folio.telemetry.ui_events.dropped` (no attributes). The relay's 120 requests per minute per user and the batch processors' queue limits still apply.
- **Sampling.** None: a single-household deployment produces too little for it to matter. If it's ever needed, sample whole sessions by hashing `session.id` on the client, so a session is kept or dropped as one.

## Following a session

`docs/observability.md` will show these queries against `homelab`/`apps`:

1. Find recent sessions: `session.start` log events by `service.name` and `folio.app.component`.
2. Breadcrumbs: logs where `session.id = X`, ordered by time (`folio.screen.view`, `folio.action`, `folio.operation.completed`).
3. Server side: trace IDs of spans with `session.id = X`, then server spans in those traces.

## Risks

- **Semconv may change.** `session.*` and `app.screen.name` are Development. The names live in one constant table per platform, so a rename is one edit.
- **Cross-process races.** At worst one extra short session, linked by `previous_id`.
- **Web log sanitizing is new.** Today logs aren't filtered. Adding an allowlist could drop the existing `folio.web.navigation` attributes if it's wrong; the e2e test asserts them.
- **Rebases.** The Apple tag additions touch many views, so they are one line each and land last in the Apple PR.

## Open questions

1. **Inactivity timeout:** 30 min (recommended) or 15 min to match Faro and Splunk?
2. **File Provider:** join without extending (recommended), or never stamp a session on File Provider traffic?
3. **Web browse scope:** the web list is server-rendered, so its screen and filter state are in the page, not in a client span. Is the screen name enough on the web, or should the server record `folio.browse.scope` on its own request span for `/documents` and `/`?
4. **Old `app.session.start`:** keep it as is (recommended), or rename it to `folio.telemetry.configured` now that `session.start` exists?
