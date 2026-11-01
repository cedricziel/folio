# Observability

Folio exports traces, metrics and logs with OpenTelemetry. Export is off until
you set an OTLP endpoint; without one Folio only writes logs to stdout. All
configuration uses the standard `OTEL_*` variables, nothing Folio-specific.

| Variable | Effect |
| --- | --- |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Collector or backend base URL. Setting it turns export on. Per-signal `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, `..._METRICS_ENDPOINT` and `..._LOGS_ENDPOINT` also work. |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | `http/protobuf` (default) or `grpc`. |
| `OTEL_EXPORTER_OTLP_HEADERS` | Comma-separated `key=value` headers, for example `authorization=Bearer <token>`. |
| `OTEL_SERVICE_NAME` | Defaults to `folio`. |
| `OTEL_RESOURCE_ATTRIBUTES` | Extra resource attributes, for example `deployment.environment.name=prod`. |
| `OTEL_SDK_DISABLED` | `true` turns export off even when an endpoint is set. |
| `OTEL_TRACES_EXPORTER`, `OTEL_METRICS_EXPORTER`, `OTEL_LOGS_EXPORTER` | `none` turns one signal off. |

`RUST_LOG` still controls verbosity and defaults to `info`. Keep API keys in a
secret store or `.env`, never in the repository. Folio loads `.env` for the
`OTEL_*` variables too.

On shutdown (SIGTERM or Ctrl+C) Folio flushes pending data before exiting.

## What is exported

**Traces**

- One server span per HTTP request, named `METHOD /route`, with
  `http.request.method`, `http.route`, `url.path`,
  `http.response.status_code` and `user_agent.original`. A W3C `traceparent`
  header from the caller is continued.
- One `job {type}` span per background job attempt with `job.id`,
  `job.type`, `job.attempt` and `document.id`; work done by the job is nested
  inside it.
- One client span per OCR or embedding call to an AI provider, with
  `gen_ai.system`, `gen_ai.operation.name` and `server.address`.
- Spans from `#[instrument]`ed functions elsewhere in the code.

**Metrics**

| Name | Type | Attributes |
| --- | --- | --- |
| `http.server.request.duration` | histogram (s) | `http.request.method`, `http.route`, `http.response.status_code` |
| `http.server.active_requests` | up-down counter | `http.request.method` |
| `folio.jobs.processed` | counter | `job.type`, `outcome` |
| `folio.jobs.failed` | counter | `job.type` |
| `folio.jobs.duration` | histogram (s) | `job.type` |
| `folio.documents` | up-down counter | none |
| `folio.ai.request.duration` | histogram (s) | `operation` (`ocr`, `embedding`), `provider`, `outcome` |
| `folio.storage.operation.duration` | histogram (s) | `operation`, `backend` (`local`, `s3`) |

`folio.documents` starts at the stored count when Folio boots and follows
uploads and deletes after that.

**Logs**

Everything written through `tracing` is exported as OTLP log records, carrying
the trace and span ids of the active span. The OpenTelemetry and HTTP
transport crates are excluded from export to avoid feedback loops.

Database query spans are not exported yet.

## Example: SignalDB

SignalDB ingests OTLP on 4317 (gRPC) and 4318 (HTTP) and needs the API key and
an explicit tenant on every request:

```sh
OTEL_EXPORTER_OTLP_ENDPOINT=http://signaldb.example.com:4318
OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
OTEL_EXPORTER_OTLP_HEADERS=authorization=Bearer <api key>,x-tenant-id=folio
```

Header keys must be lowercase. Query the data back with the Tempo, Loki and
Prometheus compatible APIs of the router, for example
`GET /tempo/api/search?tags=service.name%3Dfolio`.

## Example: OpenTelemetry Collector

```sh
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4317
OTEL_EXPORTER_OTLP_PROTOCOL=grpc
```
