# health-probes Specification

## Purpose

Unauthenticated JSON probes that let Docker, TrueNAS, and the Pangolin reverse proxy tell whether Folio is alive and ready to serve.

## Requirements

### Requirement: Liveness probe

`GET /healthz` SHALL respond 200 with `{"status":"ok"}` while the process serves requests. It SHALL NOT touch the database or any other dependency.

#### Scenario: Process is running

- **WHEN** a client requests `GET /healthz`
- **THEN** the response status is 200 with body `{"status":"ok"}`

### Requirement: Readiness probe

`GET /readyz` SHALL run every registered health check, each limited to 2 seconds. It SHALL respond 200 with `{"status":"ok","checks":{<name>:"ok"}}` when all pass, and 503 with `{"status":"unavailable","checks":{<name>:<short error>}}` when any fails or times out. The database check runs `SELECT 1`.

#### Scenario: Database reachable

- **WHEN** a client requests `GET /readyz` and the database answers
- **THEN** the response status is 200 and `checks.database` is `ok`

#### Scenario: Database unreachable

- **WHEN** a client requests `GET /readyz` and the database connection pool is closed
- **THEN** the response status is 503 and `checks.database` holds the error

### Requirement: Probes bypass authentication

Both probe routes SHALL be served without credentials and SHALL be excluded from any authentication middleware.

#### Scenario: Anonymous request

- **WHEN** a client without credentials requests `GET /healthz` or `GET /readyz`
- **THEN** the request is handled, not redirected or rejected

### Requirement: Container healthcheck

`folio-app healthcheck` SHALL request `/healthz` on `127.0.0.1` at the configured server port with a 3-second timeout, and exit 0 on a 2xx response and 1 otherwise. The container image SHALL use it as its Docker `HEALTHCHECK`.

#### Scenario: Server healthy

- **WHEN** the server answers `/healthz` with 200
- **THEN** the subcommand exits 0

#### Scenario: Server down

- **WHEN** nothing listens on the port
- **THEN** the subcommand exits 1
