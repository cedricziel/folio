# realtime-events Specification

## Purpose

A Server-Sent Events stream that tells open browsers when background processing changes a document, so the UI can refresh without polling.

## Requirements

### Requirement: Event stream endpoint

`GET /events` SHALL respond with `text/event-stream` and stay open. Each event SHALL use the event type as its SSE event name and carry a JSON body `{"type": <event type>, "document_id": <uuid>}`. The server SHALL send a keep-alive comment every 30 seconds.

#### Scenario: Subscribe

- **WHEN** a client requests `GET /events`
- **THEN** the response status is 200 with content type `text/event-stream`

#### Scenario: Idle connection

- **WHEN** no document changes for 30 seconds
- **THEN** the client still receives a keep-alive comment

### Requirement: Processing events

The background worker SHALL emit `thumbnail_ready` when a document's first thumbnail is generated, `ocr_complete` when OCR stores non-empty text, and `updated` at the end of processing when a thumbnail was generated.

#### Scenario: PDF processed with a configured provider

- **WHEN** a PDF is processed, its thumbnails are generated, and OCR returns text
- **THEN** subscribers receive `thumbnail_ready`, then `ocr_complete`, then `updated` for that document

#### Scenario: Local provider

- **WHEN** a document is processed while the provider is `local`
- **THEN** subscribers receive `thumbnail_ready` and `updated`, and no `ocr_complete`

### Requirement: Events are persisted before they are announced

Every document event SHALL be written to the `outbox_events` table before it leaves the process. Lifecycle events (created, updated, deleted) SHALL be written in the same database transaction as the change they describe, so a change cannot commit without its event and a rolled back change leaves none. A dispatcher SHALL deliver committed events to the workflow engine and to live SSE connections and mark them dispatched, retrying failures with backoff. Events SHALL survive a restart: one written but not yet dispatched is delivered after the next start, exactly once to each consumer's effect. Dispatched events SHALL be deleted after seven days.

#### Scenario: Restart before dispatch

- **WHEN** a document is created and the server stops before the dispatcher ran
- **THEN** after the next start subscribers receive `created` once and the `document.uploaded` workflows have one run each

#### Scenario: Slow subscribers

- **WHEN** a subscriber falls behind the 100-event live buffer
- **THEN** it receives the events it missed from the outbox instead of skipping them, and no workflow run is lost

### Requirement: Event ids and resume

Each SSE message SHALL carry the id of its outbox event. A client that reconnects with a `Last-Event-ID` header SHALL first receive the dispatched events after that id, at most 500 from the last hour, and then the live stream. A client that connects without the header SHALL receive only events dispatched after it connected.

#### Scenario: Reconnect

- **WHEN** a client that last saw event 10 reconnects with `Last-Event-ID: 10` after events 11 and 12 were dispatched
- **THEN** it receives events 11 and 12, then live events

### Requirement: Workflow edits are announced

Document changes made by workflow actions SHALL be written to the outbox and reach SSE clients as `updated` events. They SHALL NOT trigger workflows.

#### Scenario: Workflow adds a tag

- **WHEN** a workflow action tags a document
- **THEN** subscribers receive `updated` for it and no `document.updated` workflow runs because of it
