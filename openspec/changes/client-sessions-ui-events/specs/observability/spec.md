## MODIFIED Requirements

### Requirement: Client telemetry relay

The system SHALL expose authenticated GET `/api/telemetry/config` returning enabled status and a same-origin endpoint, without export credentials. POST `/api/telemetry/v1/{signal}` SHALL forward supported OTLP traces, metrics and logs to the configured upstream using server credentials. Session origin checks and bearer read scope SHALL apply. The relay SHALL bound body size, concurrency and request rate, refuse unsupported signals and encodings, and exclude its own export requests from telemetry. It SHALL forward the OTLP body unchanged, without decoding it or adding, removing or rewriting attributes, so client attributes such as `session.id` and `app.screen.name` reach the upstream as sent.

#### Scenario: Export is disabled

- **WHEN** client telemetry is not explicitly enabled or no compatible HTTP endpoint is configured
- **THEN** authenticated GET `/api/telemetry/config` returns 200 with `enabled=false`, and POST export returns 404

#### Scenario: Authenticated export

- **WHEN** an authenticated client posts valid OTLP to a supported signal endpoint
- **THEN** the relay forwards the body with server-owned headers and returns the upstream success response

#### Scenario: Missing authentication

- **WHEN** an unauthenticated client requests telemetry while authentication is required
- **THEN** the API returns 401 without exporting data

#### Scenario: Client attributes pass through

- **WHEN** an authenticated client sends POST `/api/telemetry/v1/traces` or `/api/telemetry/v1/logs` with OTLP whose records carry `session.id`, `app.screen.name` and `folio.action.name`
- **THEN** the relay returns 200 and the upstream receives a body identical to the one posted
