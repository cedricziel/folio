# Client sessions and UI events

## Why

Client telemetry today can show that a request happened, but not what the person was doing. Spans carry `folio.installation.id` and a per-process `service.instance.id`. Neither is a user journey: an installation lives for months, and a process is one of several (app, share extension, File Provider) that serve the same few minutes of use. The web only records `folio.navigation` spans with a cleaned route.

So a question like the last check in #209 ("open a document from Inbox, go back, switch to All documents, open another") can't be answered from SignalDB. `documents.browse` doesn't say whether it was the Inbox, All documents or a saved view, and nothing records which screen was on show.

## What changes

- **Client session ID.** While a session is live, client spans and log records carry a random `session.id` (OpenTelemetry semantic conventions, Development status). A session ends after 30 minutes without user activity, after 4 hours, on sign-out and on a server switch. `session.start` and `session.end` log events mark the boundaries, and `session.previous_id` links a session that ended by timeout or maximum length to the one that follows it. Only user activity extends or rotates a session; background uploads, Spotlight, File Provider callbacks and Turbo frame fetches never do. On Apple the app, share extension and App Intents share one session through a locked record in the App Group, and the File Provider extension stamps a live session without extending or starting one. The browser shares one session across tabs through `localStorage`.
- **Screen events.** A `folio.screen.view` log event fires when the visible screen changes. Spans and logs also carry the current `app.screen.name`. Names come from a fixed vocabulary that this change defines; anything else is reported as `other`.
- **Action events.** A `folio.action` log event records a deliberate user action (upload, delete, mark reviewed, open saved view, …) by fixed name only.
- **Browse context.** The Apple `documents.browse` span records `folio.browse.scope` (`inbox`, `all`, `saved_view`, `intent`), the kinds of filter in use, the sort and whether it is the first page. The server's `GET /`, `/documents` and `/documents/search` spans record the web scope (`inbox`, `all`, `search`) and filter kinds.
- **Page-load link.** HTML responses carry a `Server-Timing` traceparent, so a full page load's `folio.navigation` span joins the server request's trace.
- **Rename.** The Apple `app.session.start` event becomes `folio.telemetry.configured`, since it fires on every reconfigure.
- **Relay pass-through.** The relay at `/api/telemetry/v1/*` already forwards OTLP bodies byte for byte. A regression test pins that, and the docs say so.
- **Docs.** `docs/observability.md` lists the vocabulary and gives SignalDB queries that follow one session.

What doesn't change: the privacy contract. No account IDs, document IDs, document contents, titles, tag names, search text, query strings or bodies. Session IDs never go on the resource, on metric attributes, or to the server as baggage. Server spans are reached from client spans through the trace context that is already propagated.

## Capabilities

- New `client-sessions`: session lifetime, sharing across surfaces, screen and action events, browse context, fixed vocabulary and limits.
- Modified `observability`: the client relay requirement states that it forwards client attributes unchanged. Added: page loads join their server request through `Server-Timing`.

## Impact

- **Hexagonal layers.** No change to `folio-core` ports or models, and no `folio-infra` adapter change. `folio-api`: one Askama block in `templates/base.html` with a value per page template, `data-folio-action` attributes on a few forms and buttons, browse attributes recorded on the page route spans, and the `Server-Timing` header in the request-tracing middleware (`telemetry.rs`). The relay code in `client_telemetry.rs` stays as it is and its body-preservation test is extended.
- **HTTP.** No new endpoint. The HTML (Turbo) pages gain a `<meta name="folio-screen">` tag and HTML responses a `Server-Timing` header. JSON API routes and `openapi/openapi.json` are unchanged, so the generated Swift client is unchanged.
- **Settings.** None. The 30-minute and 4-hour limits are constants. `FOLIO_TELEMETRY__CLIENTS_ENABLED` still gates all client export.
- **Database.** No migration.
- **Web.** A new side-effect-free `web/telemetry-session.mjs` (session store, vocabulary, limits) with `node --test` unit tests, and wiring in `web/telemetry.mjs`. No new npm dependency.
- **Apple.** FolioKit gains a `TelemetrySession` store in the App Group (JSON record under a `flock` lock), session and screen state kept outside the rebuildable export runtime, typed `TelemetryScreen`/`TelemetryAction` enums, a `.telemetryScreen(_:)` view modifier and a browse context on `DocumentBrowseQuery`. Views get one modifier each. The Apple work stacks on #377, which adds `folio.app.component` and touches `FolioTelemetry.swift`.
