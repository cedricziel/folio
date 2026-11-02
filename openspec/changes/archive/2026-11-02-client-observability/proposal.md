## Why

The server has optional OTLP export, but browsers and native apps have no instrumentation. Hive does not currently receive Folio telemetry.

## What changes

- Add standard browser and Swift SDK instrumentation for traces, metrics and logs.
- Group resources under `service.namespace=folio`, with distinct service names, persistent installation IDs and non-identifying platform attributes.
- Add authenticated JSON configuration and OTLP relay endpoints under `/api/telemetry`. Export credentials remain server-side.
- Configure Hive to send telemetry to SignalDB tenant `homelab`, dataset `apps`.

## Impact

Changes touch folio-infra resource attributes, folio-api JSON routes/templates, browser assets and Apple clients. No database migration is needed. Client telemetry is opt-in through `FOLIO_TELEMETRY__CLIENTS_ENABLED`; endpoints and credentials use standard OTEL environment variables, with `FOLIO_TELEMETRY__OTLP_HTTP_ENDPOINT` available when server export uses gRPC. No settings-table override is added. Document/account content and authentication material are excluded from client telemetry.
