# Spec Delta

## MODIFIED Requirements

### Requirement: Event stream endpoint

`GET /events` SHALL respond to a signed-in user with `text/event-stream` and stay open. Each event SHALL use the event type as its SSE event name and carry a JSON body `{"type": <event type>, "document_id": <uuid>, "vault_id": <uuid>}`. The server SHALL send a keep-alive comment every 30 seconds.

#### Scenario: Subscribe

- **WHEN** a signed-in user requests `GET /events`
- **THEN** the response status is 200 with content type `text/event-stream`

#### Scenario: Idle connection

- **WHEN** no document changes for 30 seconds
- **THEN** the client still receives a keep-alive comment

#### Scenario: Event body

- **WHEN** a document in `Home` finishes processing
- **THEN** the `updated` event body carries the document's ID and `Home`'s ID as `vault_id`

## ADDED Requirements

### Requirement: Events are filtered by vault access

The server SHALL deliver an event to a subscriber only if the subscriber has a role in the event's vault at the time the event is sent.

#### Scenario: Subscriber without a role

- **WHEN** a document in `Home` is processed while Bob, who has no role in `Home`, is subscribed
- **THEN** Bob receives no event for that document

#### Scenario: Role revoked during the subscription

- **WHEN** Bob's grant on `Home` is revoked while his stream is open and a `Home` document is then processed
- **THEN** Bob receives no event for that document

### Requirement: Move events

When a document moves, the server SHALL send `removed` with the source `vault_id` to subscribers who can read the source vault, and `added` with the target `vault_id` to subscribers who can read the target vault.

#### Scenario: Subscriber reads only the source vault

- **WHEN** a document moves from `Home` to `Business` and Carol can read `Home` but not `Business`
- **THEN** Carol receives `removed` for the document with `Home`'s ID and nothing that names `Business`

#### Scenario: Subscriber reads both vaults

- **WHEN** a document moves from `Home` to `Business` and Alice can read both
- **THEN** Alice receives `removed` with `Home`'s ID and `added` with `Business`'s ID
