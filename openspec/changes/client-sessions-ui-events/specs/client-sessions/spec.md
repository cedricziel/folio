## ADDED Requirements

### Requirement: Client session identity

Every span and log record a browser or Apple client exports SHALL carry `session.id`, a random lowercase UUID v4. `session.id` SHALL NOT be a resource attribute or a metric attribute, and SHALL NOT be sent to the server outside telemetry export. A session SHALL end after 30 minutes without user activity, after 4 hours, on sign-out and on a server switch. Screen views, actions and the app becoming active SHALL count as user activity. Network requests and File Provider callbacks SHALL NOT. Expiry SHALL be checked when the session is read, not with timers.

#### Scenario: Activity keeps the session

- **WHEN** a person changes screens every 10 minutes for an hour
- **THEN** every exported span and log record in that hour carries the same `session.id`

#### Scenario: Inactivity rotates the session

- **WHEN** the next screen view comes more than 30 minutes after the last activity
- **THEN** a `session.end` event with `folio.session.end_reason=timeout` is emitted for the old ID, and a `session.start` event with a new `session.id` and `session.previous_id` set to the old ID is emitted before the screen view

#### Scenario: Maximum length

- **WHEN** a session with continuous activity reaches 4 hours
- **THEN** the next read ends it with `folio.session.end_reason=max_duration` and starts a linked session

#### Scenario: Sign-out

- **WHEN** a person signs out or switches server
- **THEN** `session.end` is emitted with `sign_out` or `server_change`, the stored session is deleted, and the next session's `session.start` has no `session.previous_id`

#### Scenario: Reload in the browser

- **WHEN** a browser tab reloads, or a second tab opens, within 30 minutes of activity
- **THEN** it keeps the same `session.id`

#### Scenario: Browser storage blocked

- **WHEN** `localStorage` is unavailable
- **THEN** the page uses an in-memory session and normal use continues without errors

### Requirement: Shared session across Apple surfaces

The Apple app, share extension, File Provider extension and App Intents SHALL share one session through a single record in the App Group. A surface SHALL read the record before stamping and adopt a live session instead of starting its own. The app, intents and the share extension SHALL extend the session they use. The File Provider extension SHALL use a live session without extending it, and SHALL start one only when none is live. Records from each process SHALL still carry its `folio.app.component` resource attribute.

#### Scenario: Share extension joins the app session

- **WHEN** the share extension starts within 30 minutes of activity in the app
- **THEN** its spans and logs carry the app's `session.id` and `folio.app.component=share`

#### Scenario: App adopts a session the extension started

- **WHEN** the share extension started a session and the app becomes active 5 minutes later
- **THEN** the app uses that `session.id` and emits no new `session.start`

#### Scenario: File Provider does not keep a session alive

- **WHEN** the File Provider extension serves requests for 40 minutes after the last app activity
- **THEN** the session times out 30 minutes after that activity, and later File Provider records carry a new linked `session.id`

#### Scenario: Two processes start a session at once

- **WHEN** two processes find no live session and both write a new one
- **THEN** both use the record that was stored last from their next read onward

### Requirement: Screen events

A client SHALL emit a `folio.screen.view` log event with `app.screen.name` when the visible screen changes. It SHALL also stamp the current `app.screen.name` on later spans and log records. Screen names SHALL come from the screen vocabulary in the design. Any other value SHALL be reported as `other`. A repeat of the same screen within one second SHALL NOT emit another event. Sign-in and setup pages SHALL emit nothing.

#### Scenario: Inbox to document and back

- **WHEN** a person opens a document from the Inbox, goes back and switches to All documents
- **THEN** the session's events show `inbox`, `document.detail`, `inbox`, `documents` in order

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

A client SHALL emit a `folio.action` log event with `folio.action.name` from the action vocabulary when a person performs a listed action. Qualifiers SHALL be limited to `folio.upload.source` and `folio.intent.name`, with their listed values. On the web, actions SHALL be declared with a `data-folio-action` attribute, and a name outside the vocabulary SHALL emit nothing. App Intents SHALL emit `intent.run` with `folio.intent.name`.

#### Scenario: Delete from the web

- **WHEN** a person submits a form marked `data-folio-action="document.delete"`
- **THEN** a `folio.action` event with `folio.action.name=document.delete` and the current `app.screen.name` is exported, with no document ID, title or URL

#### Scenario: Unlisted action

- **WHEN** an element carries `data-folio-action="document.secret"`
- **THEN** no action event is emitted

#### Scenario: Shortcut runs

- **WHEN** the Add Tag shortcut runs
- **THEN** a `folio.action` event with `folio.action.name=intent.run` and `folio.intent.name=add_tag` is exported, without the tag or document

### Requirement: Browse context

The Apple `documents.browse` span and its completion log SHALL carry `folio.browse.scope` (`inbox`, `all`, `saved_view`, `intent`), `folio.browse.filters` (a sorted array of the filter kinds in use, from `text`, `tag`, `correspondent`, `document_type`, `document_date`, `created_date`, `reviewed`), `folio.browse.sort` and `folio.browse.page` (`first` or `next`). Filter values SHALL NOT be exported. The context SHALL NOT be sent to the browse API.

#### Scenario: Inbox browse

- **WHEN** the Inbox loads its first page with no filters
- **THEN** the span carries `folio.browse.scope=inbox`, an empty `folio.browse.filters`, `folio.browse.sort=newest` and `folio.browse.page=first`

#### Scenario: Filtered browse

- **WHEN** All documents loads the next page filtered by a tag and search text
- **THEN** the span carries `folio.browse.scope=all`, `folio.browse.filters=["tag","text"]` and `folio.browse.page=next`, and no tag ID or search text

### Requirement: Client telemetry privacy and limits

New client attributes SHALL use only the vocabulary in this change. Clients SHALL NOT export account IDs, document IDs, document contents, titles, tag or correspondent names, search text, query strings, request or response bodies, DOM selectors or tap coordinates. Each process SHALL emit at most 60 screen and action events per minute and drop the rest, counting drops in `folio.telemetry.ui_events.dropped`. Telemetry SHALL stay disabled until the server's telemetry configuration enables it.

#### Scenario: Burst of actions

- **WHEN** a client records 100 actions within one minute
- **THEN** 60 action events are exported and the drop counter increases by 40

#### Scenario: Telemetry disabled

- **WHEN** the server reports client telemetry disabled
- **THEN** no session, screen or action data leaves the client, and the app behaves normally
