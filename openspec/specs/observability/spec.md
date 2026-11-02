# observability Specification

## Purpose

Exporting traces, metrics and logs with OpenTelemetry, configured only through the standard `OTEL_*` environment variables.
## Requirements
### Requirement: Export is opt-in

The system SHALL export telemetry only when an OTLP endpoint is configured through `OTEL_EXPORTER_OTLP_ENDPOINT` or a per-signal endpoint variable, and SHALL NOT export when `OTEL_SDK_DISABLED` is `true`. Without an endpoint it SHALL write plain logs to stdout. It SHALL honor `OTEL_EXPORTER_OTLP_PROTOCOL` (`http/protobuf` or `grpc`), `OTEL_EXPORTER_OTLP_HEADERS`, `OTEL_SERVICE_NAME` (default `folio`) and `OTEL_RESOURCE_ATTRIBUTES`, and SHALL flush pending data on graceful shutdown.

#### Scenario: No endpoint

- **WHEN** Folio starts without any OTLP endpoint variable
- **THEN** no exporter is created and logs go to stdout

#### Scenario: Endpoint set

- **WHEN** Folio starts with `OTEL_EXPORTER_OTLP_ENDPOINT=http://collector:4318`
- **THEN** traces, metrics and logs are exported as `http/protobuf` with service name `folio`

### Requirement: Request, job and provider spans

The system SHALL create a server span for every HTTP request named `<method> <route>` with `http.request.method`, `http.route` and `http.response.status_code`, continuing an incoming `traceparent`. It SHALL create a span per background job attempt carrying the job type and document id, and a client span per OCR or embedding call to an AI provider.

#### Scenario: Request to a parameterized route

- **WHEN** a client calls `GET /api/documents/<id>`
- **THEN** a server span named `GET /api/documents/{id}` with `http.route` `/api/documents/{id}` is recorded

### Requirement: Metrics

The system SHALL record `http.server.request.duration`, `folio.jobs.processed`, `folio.jobs.failed`, `folio.jobs.duration`, `folio.documents`, `folio.ai.request.duration` and `folio.storage.operation.duration` with the attributes documented in `docs/observability.md`.

#### Scenario: Request duration

- **WHEN** any HTTP request completes
- **THEN** `http.server.request.duration` records its duration with method, route and status code

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
