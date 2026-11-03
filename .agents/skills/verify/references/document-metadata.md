## Verify document dates

Run `FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test document_dates`
with the isolated build settings above. The SQLite and PostgreSQL contracts
cover historical calendar dates, unchanged import time, omission and clearing,
actor attribution, no-op suppression, and rollback when outbox insertion fails.
They verify one generic update shares the date event's mutation and correlation.
Run `cargo test -p folio-api --test document_dates --test openapi --test vaults`
for JSON validation, scoped web redirects, and viewer permission enforcement.
These fixtures use disposable databases and in-process requests.

## Verify document metadata transitions

Run `FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test document_transitions --test repository_contract --test workflows`
with a task-owned temporary target directory. These suites use temporary SQLite
databases, disposable PostgreSQL containers, and loopback webhook fixtures.
They cover exact title, classification, and tag deltas, no-ops, rollback,
server-derived actors, shared mutation identities, and compatibility updates.
Move contracts check source/destination snapshot isolation, rejected vault moves,
and same-vault no-ops on both databases.
Domain-only outbox rows have no legacy document projection; assertions about
compatibility updates must select that projection explicitly.

Run `cargo test -p folio-api --test routes classification_form` for the HTML
and API form entry points and malformed-ID preservation. Run
`cargo test -p folio-mcp --test tools update_document_changes_fields` for MCP
metadata edits and the persisted title delta. Both use isolated fixtures.

