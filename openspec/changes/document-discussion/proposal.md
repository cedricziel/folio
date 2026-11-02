# Document notes and comments

## Why

Readers need editable annotations and a chronological discussion attached to a document. Neither exists today.

## What changes

Add separate Notes and Comments sections to document details in the web and Apple apps. Both use one entry model with an immutable kind. Notes capture information about the document. Comments capture authored discussion, oldest first. Replies are outside this change because there is no existing threaded discussion UX.

## Impact

- folio-core: entry models, bounded page queries, and a repository port.
- folio-infra: SQLite and PostgreSQL adapters and new `20261102163000_document_discussion.sql` migrations. This slice reserves versions 20261102163000 through 20261102163999, after the foundation migrations; check main for unrelated collisions before creating them. Existing documents receive no fabricated entries.
- folio-api: JSON CRUD routes and HTML form routes, OpenAPI, escaped templates, and authorization.
- Apple: generated client, service methods, state, and accessible read/add/edit/delete views.
- MCP: preserve existing document response compatibility. No new MCP write tools in this slice.

Publish note/comment creation, update, and deletion events through the existing transactional outbox, with selectable triggers in web and Apple workflows. Use the foundation event envelope and exclude body text from event payloads.

No instance settings are added. Foundation lifecycle integration must merge first.
