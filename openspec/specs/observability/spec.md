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
