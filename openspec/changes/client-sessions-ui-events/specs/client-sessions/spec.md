## ADDED Requirements

### Requirement: Client session identity

While a session is live, every span and log record a browser or Apple client exports SHALL carry `session.id`, a random lowercase UUID v4. `session.id` SHALL NOT be a resource attribute or a metric attribute, and SHALL NOT be sent to the server outside telemetry export.

A session SHALL end after 30 minutes without user activity, after 4 hours, on sign-out and on a server switch. User activity SHALL be limited to:

- a screen view
- an action
- the app becoming active
- the share extension opening

Network requests, background uploads, Spotlight indexing, File Provider callbacks, Turbo frame and event-stream fetches and telemetry reconfiguration SHALL NOT extend, rotate or start a session.

Expiry SHALL be checked when the session is read, not with timers. A session whose stored times are later than the current clock SHALL count as expired.

#### Scenario: Activity keeps the session

- **WHEN** a person changes screens every 10 minutes for an hour
- **THEN** every exported span and log record in that hour carries the same `session.id`

#### Scenario: Inactivity rotates the session

- **WHEN** the next screen view comes more than 30 minutes after the last activity
- **THEN** a `session.end` event is emitted for the old ID with `folio.session.end_reason=timeout`
- **AND** a `session.start` event with a new `session.id`, and `session.previous_id` set to the old ID, is emitted before the screen view

#### Scenario: Maximum length

- **WHEN** a session with continuous activity reaches 4 hours
- **THEN** the next activity ends it with `folio.session.end_reason=max_duration` and starts a session whose `session.previous_id` is the old ID

#### Scenario: Background work after the timeout

- **WHEN** a background upload or a Turbo frame fetch runs 40 minutes after the last activity
- **THEN** no `session.start` or `session.end` is emitted, and the stored session is unchanged

#### Scenario: Clock set back

- **WHEN** the device clock is earlier than the session's last activity at the next activity
- **THEN** the session ends with `folio.session.end_reason=timeout` and a linked session starts

#### Scenario: Sign-out

- **WHEN** a person signs out or switches server
- **THEN** `session.end` with `sign_out` or `server_change` is emitted and flushed before credentials are removed, on a best-effort basis
- **AND** the stored session is deleted, and the next session's `session.start` has no `session.previous_id`

#### Scenario: Reload in the browser

- **WHEN** a browser tab reloads, or a second tab opens, within 30 minutes of activity
- **THEN** it keeps the same `session.id`

#### Scenario: Browser storage blocked

- **WHEN** `localStorage` is unavailable
- **THEN** the page uses an in-memory session, and normal use continues without errors

### Requirement: Shared session across Apple surfaces

The Apple app, share extension, File Provider extension and App Intents SHALL share one session through a single record in the App Group container. A surface SHALL read the record under an exclusive cross-process lock, and SHALL adopt a live session instead of starting its own.

The app, intents and the share extension SHALL extend the session they use, or rotate it when it has expired. The File Provider extension SHALL stamp a live session without extending it, and SHALL NOT start or rotate one. Records from each process SHALL still carry its `folio.app.component` resource attribute.

The record SHALL carry a sign-in epoch. A surface SHALL NOT adopt a session whose epoch differs from the one it last saw.

#### Scenario: Share extension joins the app session

- **WHEN** the share extension starts within 30 minutes of activity in the app
- **THEN** its spans and logs carry the app's `session.id` and `folio.app.component=share`

#### Scenario: App adopts a session the extension started

- **WHEN** the share extension started a session and the app becomes active 5 minutes later
- **THEN** the app uses that `session.id` and emits no new `session.start`

#### Scenario: File Provider without a live session

- **WHEN** the File Provider extension serves requests 40 minutes after the last activity
- **THEN** its records carry no `session.id`, and no session is started

#### Scenario: Extension running across sign-out

- **WHEN** an extension that adopted a session keeps running while the person signs out and signs in again
- **THEN** its next records don't carry the old `session.id`

#### Scenario: Two processes rotate at once

- **WHEN** two processes find the same expired session at the same time
- **THEN** exactly one `session.start` is emitted, and both processes use the new `session.id`

### Requirement: Screen events

A client SHALL emit a `folio.screen.view` log event with `app.screen.name` when the visible screen changes, and SHALL stamp the current `app.screen.name` on later spans and log records. Screen names SHALL come from the screen vocabulary in the design; any other value SHALL be reported as `other`. A repeat of the same screen within one second SHALL NOT emit another event.

Sign-in and setup pages, and partials rendered into Turbo frames, SHALL emit nothing. On Apple, a screen shown before telemetry is configured SHALL be emitted once the export runtime is ready.

#### Scenario: Inbox to document and back

- **WHEN** a person opens a document from the Inbox, goes back and switches to All documents
- **THEN** the session's events show `inbox`, `document.detail`, `inbox`, `documents` in order

#### Scenario: Cold launch on Apple

- **WHEN** the app launches on the Inbox before `/api/telemetry/config` has answered
- **THEN** once telemetry is enabled, `session.start` and a `folio.screen.view` for `inbox` are exported

#### Scenario: Web page screen

- **WHEN** a web page loads, directly or through a Turbo visit
- **THEN** the `folio.screen.view` event carries the screen name from the page's `folio-screen` meta tag

#### Scenario: Unknown screen name

- **WHEN** a page or view supplies a name that isn't in the vocabulary
- **THEN** the event carries `app.screen.name=other`

#### Scenario: Network span records its screen

- **WHEN** a request is made while the Inbox is on screen
- **THEN** its client span carries `app.screen.name=inbox`

### Requirement: Action events

A client SHALL emit a `folio.action` log event with `folio.action.name` from the action vocabulary when a person performs a listed action. Qualifiers SHALL be limited to `folio.upload.source` and `folio.intent.name`, with their listed values.

On the web, actions SHALL be declared with a `data-folio-action` attribute, and a name outside the vocabulary SHALL emit nothing. Pending telemetry SHALL be flushed with `keepalive` on `pagehide`, and after an action on a form that leaves the page. App Intents SHALL emit `intent.run` with `folio.intent.name`.

On Apple, actions raised before the export runtime is ready SHALL be dropped.

#### Scenario: Delete from the web

- **WHEN** a person submits a form marked `data-folio-action="document.delete"`, which leaves the page
- **THEN** a `folio.action` event with `folio.action.name=document.delete` and the current `app.screen.name` is exported
- **AND** it carries no document ID, title or URL

#### Scenario: Unlisted action

- **WHEN** an element carries `data-folio-action="document.secret"`
- **THEN** no action event is emitted

#### Scenario: Shortcut runs

- **WHEN** the Add Tag shortcut runs
- **THEN** a `folio.action` event with `folio.action.name=intent.run` and `folio.intent.name=add_tag` is exported, without the tag or document

### Requirement: Browse context

The Apple `documents.browse` span and its completion log SHALL carry:

- `folio.browse.scope`: `inbox`, `all`, `saved_view` or `intent`
- `folio.browse.filters`: a sorted array of the filter kinds in use, from `text`, `tag`, `correspondent`, `document_type`, `document_date`, `created_date` and `reviewed`
- `folio.browse.sort`
- `folio.browse.page`: `first` or `next`

The server request spans for GET `/`, GET `/documents` and GET `/documents/search` SHALL carry `folio.browse.scope` (`inbox`, `all` or `search`) and `folio.browse.filters`.

Filter values SHALL NOT be exported. Browse context SHALL NOT be a metric attribute. The Apple browse context SHALL NOT be sent to the browse API.

#### Scenario: Inbox browse on Apple

- **WHEN** the Inbox loads its first page with no filters
- **THEN** the span carries `folio.browse.scope=inbox`, an empty `folio.browse.filters`, `folio.browse.sort=newest` and `folio.browse.page=first`

#### Scenario: Filtered browse on Apple

- **WHEN** All documents loads the next page filtered by a tag and search text
- **THEN** the span carries `folio.browse.scope=all`, `folio.browse.filters=["tag","text"]` and `folio.browse.page=next`
- **AND** it carries no tag ID or search text

#### Scenario: Web Inbox and All documents

- **WHEN** the server handles GET `/` without `view`, then GET `/?view=all&tag_id=…`, both returning 200
- **THEN** the first request span carries `folio.browse.scope=inbox`
- **AND** the second carries `folio.browse.scope=all` and `folio.browse.filters=["tag"]`, without the tag ID

### Requirement: Client telemetry privacy and limits

New client attributes SHALL use only the vocabulary in this change. Clients SHALL NOT export:

- account IDs, document IDs or document contents
- titles, or tag or correspondent names
- search text, query strings, or request or response bodies
- DOM selectors or tap coordinates

UI events SHALL be log records with an event name. They SHALL NOT create spans or metric points.

Each process SHALL emit at most 60 screen and action events per minute and drop the rest, counting drops in `folio.telemetry.ui_events.dropped`. Telemetry SHALL stay disabled until the server's telemetry configuration enables it.

The Apple event formerly named `app.session.start` SHALL be emitted as `folio.telemetry.configured`.

#### Scenario: Burst of actions

- **WHEN** a client records 100 actions within one minute
- **THEN** 60 action events are exported and the drop counter increases by 40

#### Scenario: Metrics stay low-cardinality

- **WHEN** an Apple operation completes within a session
- **THEN** its `folio.app.operations` metric point carries only `folio.operation` and `folio.outcome`

#### Scenario: Telemetry disabled

- **WHEN** the server reports client telemetry disabled
- **THEN** no session, screen or action data leaves the client, and the app behaves normally
