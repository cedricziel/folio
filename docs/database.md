# Database

Folio runs on SQLite (the default) or PostgreSQL. The backend follows the
scheme of `FOLIO_DATABASE__URL`:

| URL | Backend |
| --- | --- |
| `sqlite:folio.db?mode=rwc` | SQLite (default) |
| `postgres://user:pass@host:5432/folio` | PostgreSQL 14 or newer |
| `postgresql://user:pass@host/folio?sslmode=require` | PostgreSQL with TLS |

There is no data migration between backends. Pick one for a new installation.

## Layout

Each backend has its own repository implementations behind the same ports in
`folio-core` (`crates/folio-infra/src/database/sqlite` and `.../postgres`), and
its own migrations:

```
migrations/sqlite/     001_initial.sql  002_jobs.sql  ...
migrations/postgres/   001_initial.sql  002_jobs.sql  ...
```

`001` to `004` keep their sequence numbers. New migrations use a timestamp
prefix, `YYYYMMDDHHMMSS_name.sql`, so parallel branches do not collide on the
next number. A schema change adds the same file name to both directories. PostgreSQL uses native types (`uuid`, `timestamptz`, `jsonb`,
`text`); SQLite keeps text columns.

Queries are plain `sqlx::query_as` calls with `FromRow` structs, not `query!`
macros. Building therefore needs no database and no offline query cache for
either backend.

## PostgreSQL

Migrations run on startup. The role needs permission to create tables, and to
run `CREATE EXTENSION vector` if you want pgvector ranking (see below).

### Semantic search

Embeddings are always stored in `document_embeddings.embedding` (`real[]`).

- With the [pgvector](https://github.com/pgvector/pgvector) extension available,
  Folio runs `CREATE EXTENSION IF NOT EXISTS vector` at startup, adds an
  `embedding_vec vector` column, backfills it, and ranks by cosine distance in
  the database. Use an image such as `pgvector/pgvector:pg17`.
- Without it, Folio loads the vectors and ranks by cosine similarity in Rust,
  as the SQLite backend does. This is fine for personal-scale collections.

The startup log says which mode is active.

## Job leases

SQLite and PostgreSQL assign each claimed job a unique claim ID and a
five-minute lease, using the database clock. Workers renew the lease every
minute while processing documents or workflow runs. Recovery requeues expired
leases, including jobs whose owner crashed. A long-running job keeps its claim
while renewal succeeds, regardless of when it started.

Renewal, completion, and failure updates require the current, unexpired claim.
An expired worker cannot change the queue state of a replacement attempt. Each
renewal has a one-minute deadline. If renewal fails, times out, or loses ownership,
the worker cancels its processing future.
PostgreSQL uses `FOR UPDATE SKIP LOCKED` when claiming work. SQLite claims work
with an atomic update.

Delivery remains at least once. Cancellation cannot undo writes or external
requests already completed by an earlier attempt. Blocking local embedding
computations can continue after cancellation. Consumers must tolerate retries.
Leases do not provide exactly-once external effects.

Stop all old worker instances before starting upgraded binaries. Older binaries
do not renew leases or check claim IDs.

## Docker Compose

`docker-compose.example.yml` runs Folio next to PostgreSQL with secrets read
from files.

## Tests

`cargo test -p folio-infra` runs one shared contract suite
(`crates/folio-infra/tests/repository_contract.rs`) against SQLite and against
PostgreSQL in a testcontainers container (`pgvector/pgvector:pg17`, plus
`postgres:17-alpine` for the no-extension path). Container tests need a Docker
daemon. Without one they print `SKIPPED` and pass, unless
`FOLIO_REQUIRE_DOCKER=1` is set, in which case they fail. CI sets it.
