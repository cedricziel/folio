## Verify document notes and comments

Run `RUSTC_WRAPPER= FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test document_entries --test document_entry_schema --test domain_events --test repository_contract`.
These disposable SQLite and PostgreSQL 17 suites cover scoped entry persistence,
creation receipts, concurrent retries, event rollback, no-op mutations, all six
body-free transitions, actor/lineage attribution, moves, and receipt purge.

Run `RUSTC_WRAPPER= cargo test -p folio-api --test document_entries --test entry_pages --test mutation_actor --test personal_tokens`.
The request fixtures cover contributor access to another member's document,
author editing, moderation, revoked roles, personal-token scopes, scoped cursors,
deleted creation retries, escaped HTML, and retained failed forms.
Regenerate the linked native API schema with
`RUSTC_WRAPPER= UPDATE_OPENAPI=1 cargo test -p folio-api --test openapi`.

Build `folio-app`, then run `npx playwright test discussion.spec.ts` from `e2e/`
with the disposable setup and teardown. Both viewport cases commit a creation
while dropping its response, retry the retained key/body, then edit and delete.
They also verify separate sections and run scoped accessibility checks. Never
set `E2E_BASE_URL` to an existing archive.
