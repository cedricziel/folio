# Client sessions and UI events

## Why

Client telemetry today can show that a request happened, but not what the person was doing. Spans carry `folio.installation.id` and a per-process `service.instance.id`. Neither is a user journey: an installation lives for months, and a process is one of several (app, share extension, File Provider) that serve the same few minutes of use. The web only records `folio.navigation` spans with a cleaned route.

So a question like the last check in #209 ("open a document from Inbox, go back, switch to All documents, open another") can't be answered from SignalDB. `documents.browse` doesn't say whether it was the Inbox, All documents or a saved view, and nothing records which screen was on show.

## What changes

- **Client session ID.** Every client span and log record carries a random `session.id` (OpenTelemetry semantic conventions, Development status). A session ends after 30 minutes without user activity, after 4 hours, on sign-out and on a server switch. `session.start` and `session.end` log events mark the boundaries, and `session.previous_id` links a timed-out session to the one that follows it. On Apple the app, share extension, File Provider extension and App Intents share one session through the App Group. The browser shares one session across tabs through `localStorage`.
- **Screen events.** A `folio.screen.view` log event fires when the visible screen changes. Spans and logs also carry the current `app.screen.name`. Names come from a fixed vocabulary that this change defines; anything else is reported as `other`.
- **Action events.** A `folio.action` log event records a deliberate user action (upload, delete, mark reviewed, open saved view, …) by fixed name only.
- **Browse context.** The Apple `documents.browse` span records `folio.browse.scope` (`inbox`, `all`, `saved_view`, `intent`), the kinds of filter in use, the sort and whether it is the first page.
- **Relay pass-through.** The relay at `/api/telemetry/v1/*` already forwards OTLP bodies byte for byte. A regression test pins that, and the docs say so.
- **Docs.** `docs/observability.md` lists the vocabulary and gives SignalDB queries that follow one session.

What doesn't change: the privacy contract. No account IDs, document IDs, document contents, titles, tag names, search text, query strings or bodies. Session IDs never go on the resource, on metric attributes, or to the server as baggage. Server spans are reached from client spans through the trace context that is already propagated.

## Capabilities

- New `client-sessions`: session lifetime, sharing across surfaces, screen and action events, browse context, fixed vocabulary and limits.
- Modified `observability`: the client relay requirement states that it forwards client attributes unchanged.

## Impact

- **Hexagonal layers.** No change to `folio-core` ports or models, and no `folio-infra` adapter change. `folio-api`: one Askama block in `templates/base.html` with a value per page template, plus `data-folio-action` attributes on a few forms and buttons. The relay code in `client_telemetry.rs` stays as it is and gains a test.
- **HTTP.** No new endpoint. The HTML (Turbo) pages gain a `<meta name="folio-screen">` tag. JSON API routes and `openapi/openapi.json` are unchanged, so the generated Swift client is unchanged.
- **Settings.** None. The 30-minute and 4-hour limits are constants. `FOLIO_TELEMETRY__CLIENTS_ENABLED` still gates all client export.
- **Database.** No migration.
- **Web.** `web/telemetry.mjs` gains about 150 lines (session store, stamping, screen and action events, limits) with no new npm dependency.
- **Apple.** FolioKit gains a `TelemetrySession` store in the App Group, typed `TelemetryScreen`/`TelemetryAction` enums, a `.telemetryScreen(_:)` view modifier and a browse context on `DocumentBrowseQuery`. Views get one modifier each. The Apple work stacks on #377, which adds `folio.app.component` and touches `FolioTelemetry.swift`.
