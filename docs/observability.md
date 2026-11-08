# Observability

Folio exports traces, metrics and logs with OpenTelemetry. Export is off until
you set an OTLP endpoint; without one Folio only writes logs to stderr. Server
export uses the standard `OTEL_*` variables. Client export has an additional
explicit opt-in described below.

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
- `GET /`, `GET /documents` and `GET /documents/search` spans also carry
  `folio.browse.scope` (`inbox`, `all` or `search`) and
  `folio.browse.filters`, the sorted kinds of filter in use (`reviewed`,
  `document_date`, `correspondent`, `document_type`, `tag`, `text`). Filter
  values are never recorded.
- HTML responses carry `Server-Timing: traceparent;desc="00-<trace>-<span>-01"`,
  naming the request span so a browser page load can join its trace. JSON API
  responses don't.
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

## Browser and native clients

Set `FOLIO_TELEMETRY__CLIENTS_ENABLED=true` to enable client telemetry. Clients
fetch authenticated `/api/telemetry/config` and export through the same-origin
`/api/telemetry/v1/traces`, `/metrics` and `/logs` routes. Server export headers
and credentials stay on the server. Session origin checks apply; bearer tokens
need `documents:read`, including for telemetry POST requests.

The relay uses the HTTP OTLP endpoint and headers above. If the server exports
over gRPC, set `FOLIO_TELEMETRY__OTLP_HTTP_ENDPOINT` to the HTTP collector base
URL for client forwarding. Export requests are limited to one MiB, four
concurrent upstream requests and 120 requests per minute per authenticated user,
with a shared 1,200-request budget. Compressed bodies and redirects are refused.
Payload reads and upstream requests have five-second deadlines. Telemetry
requests do not create server request spans or metrics, avoiding export loops.

All components use `service.namespace=folio`. Browser resources use
`service.name=folio-web`; Apple clients use `folio-app`. Set the deployed server
name with `OTEL_SERVICE_NAME=folio-server`. Each runtime gets a distinct
`service.instance.id`. Client `folio.installation.id` values persist in local
browser storage or the Apple Keychain. Apple clients share the Keychain UUID
with extensions through the configured access group and migrate the existing
UUID from shared preferences without changing it. The Keychain item does not
sync through iCloud or move to another device in a backup. A resolved UUID stays
cached for the process. If the Keychain is unavailable before an ID can be
resolved, Apple telemetry waits until a later configuration attempt. It does
not emit a replacement ID. Restricted browser storage falls back to a runtime
ID. Set a stable server installation ID through
`OTEL_RESOURCE_ATTRIBUTES=folio.installation.id=<random UUID>`.

Browser resources describe the browser, OS and generic device class where
available. Apple resources include OS version, architecture and generic Apple
device model where available. Clients do not collect account IDs, hardware
serial numbers, advertising IDs or document contents. Client network telemetry
removes query strings, fragments and document IDs, never captures request or
response bodies or authorization headers, and only propagates trace context
to the configured Folio origin. Export failures do not interrupt app requests.

For the Hive deployment, the destination is tenant `homelab`, dataset `apps`:

```sh
OTEL_SERVICE_NAME=folio-server
OTEL_RESOURCE_ATTRIBUTES=service.namespace=folio,deployment.environment.name=production,folio.installation.id=<persisted UUID>
OTEL_EXPORTER_OTLP_ENDPOINT=http://<hive-host>:4318
OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
OTEL_EXPORTER_OTLP_HEADERS=authorization=Bearer%20<ingest-key>,x-tenant-id=homelab,x-dataset-id=apps
FOLIO_TELEMETRY__CLIENTS_ENABLED=true
```

Use a key restricted to `apps` with traces, logs and metrics write scopes.
Keep read/query credentials separate. SDK exporters send actual OTLP requests;
Swift SDKs use protobuf and browser SDKs use JSON. Neither client receives the
SignalDB key. The Swift transport obtains the current Folio OAuth token and
stops sending after sign-out.

## Client sessions, screens and actions

Browser telemetry groups a person's activity into sessions. While a session is
live, every browser span and log record carries `session.id`, a random UUID.
It is never a resource or metric attribute and is never sent to the server
outside telemetry export.

- **Lifetime.** A session ends after 30 minutes without activity, after 4
  hours, or on sign-out. Screen views and actions count as activity; network
  requests and Turbo frame fetches don't. A clock set back ends the session.
- **Sharing.** Tabs share one session through `localStorage`. With storage
  blocked, each page keeps its own.
- **Boundaries.** `session.start` and `session.end` log events mark them.
  `session.end` carries `folio.session.end_reason` (`timeout`,
  `max_duration` or `sign_out`). A session that follows a `timeout` or
  `max_duration` has `session.previous_id`. Sign-out flushes `session.end`
  best-effort before the logout request; the sign-in page then forgets the
  session, so the next one isn't linked.
- **Screens.** Each page declares its screen in
  `<meta name="folio-screen">`. A `folio.screen.view` log event with
  `app.screen.name` fires when the screen changes, and later spans and logs
  carry the current `app.screen.name`. Names come from a fixed list in
  `web/telemetry-session.mjs`; anything else becomes `other`.
- **Actions.** Elements marked `data-folio-action`, and scripts that dispatch a
  `folio:action` event, record a `folio.action` log event with
  `folio.action.name`. Names outside the list are ignored. Nothing else is read
  from the page.
- **Limits.** A repeated screen within a second is dropped, and each page sends
  at most 60 screen and action events per minute. The excess is counted in
  `folio.telemetry.ui_events.dropped`.
- **Page loads.** The first `folio.navigation` span of a full page load uses
  the server's `Server-Timing` traceparent as its parent, so it shares the
  server request's trace. Turbo visits already propagate `traceparent`.

The attribute names follow OpenTelemetry semantic conventions v1.45.0, where
`session.*` and `app.screen.name` are still in Development.

To follow one person's session in SignalDB (`homelab`/`apps`):

1. Find recent sessions: log events named `session.start` for
   `service.name=folio-web`.
2. List the breadcrumbs: log records with that `session.id`, ordered by time
   (`folio.screen.view`, `folio.action`, `session.end`).
3. Reach the server: take the trace IDs of spans with that `session.id` and
   look up `service.name=folio-server` spans with the same trace IDs.
