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

### Requirement: Slow subscribers miss events

Events SHALL be broadcast through a buffer of 100 events. A subscriber that falls behind SHALL skip the events it missed instead of disconnecting, and events sent while nobody is subscribed SHALL be dropped.

#### Scenario: No subscribers

- **WHEN** a document is processed while no client is connected
- **THEN** its events are discarded and are not replayed to later subscribers
