## ADDED Requirements

### Requirement: Client telemetry relay

The system SHALL expose authenticated GET `/api/telemetry/config` returning enabled status and a same-origin endpoint, without export credentials. POST `/api/telemetry/v1/{signal}` SHALL forward supported OTLP traces, metrics and logs to the configured upstream using server credentials. Session origin checks and bearer read scope SHALL apply. The relay SHALL bound body size, concurrency and request rate, refuse unsupported signals and encodings, and exclude its own export requests from telemetry.

#### Scenario: Export is disabled

- **WHEN** client telemetry is not explicitly enabled or no compatible HTTP endpoint is configured
- **THEN** authenticated GET `/api/telemetry/config` returns 200 with `enabled=false`, and POST export returns 404

#### Scenario: Authenticated export

- **WHEN** an authenticated client posts valid OTLP to a supported signal endpoint
- **THEN** the relay forwards the body with server-owned headers and returns the upstream success response

#### Scenario: Missing authentication

- **WHEN** an unauthenticated client requests telemetry while authentication is required
- **THEN** the API returns 401 without exporting data

### Requirement: Resource identity and client correlation

Browser, native and server telemetry SHALL use `service.namespace=folio` and distinct service names. Clients SHALL have a persistent random installation ID and a separate runtime instance ID. Available OS, browser, architecture and generic device model attributes SHALL be included without account identifiers, hardware serial numbers or advertising IDs. Client network spans SHALL propagate W3C trace context only to the configured Folio origin, exclude telemetry exports, and omit query strings, fragments, document IDs and authentication material.

#### Scenario: Browser reload

- **WHEN** a browser reloads with persistent storage available
- **THEN** its installation ID remains stable and runtime instance ID changes

#### Scenario: External request

- **WHEN** a client contacts an unrelated origin
- **THEN** no Folio trace context or telemetry export credential is attached

#### Scenario: Export unavailable

- **WHEN** telemetry configuration, authentication or the receiver is unavailable
- **THEN** normal application requests continue without telemetry UI errors
