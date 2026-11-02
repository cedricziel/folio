# Design

The detailed implementation sequence and compatibility decisions live in `docs/document-workspace-plan.md`.

## Paged queries

Add a separate paged repository method; retain unbounded `list` for existing internal callers until each migrates deliberately. A page has a validated limit between 1 and 100, a typed sort, the existing filters, and an optional continuation. Read one extra record to determine whether another page exists.

Chronological sorting uses `(created_at, id)`; title sorting uses `(title, id)` with an explicitly consistent binary collation across database backends. Every continuation binds the vault, normalized filters, and sort. Reject a continuation for a different query. A cursor is not an authorization token. Recheck access on every HTTP request.

Documents may change between pages. Deduplicate IDs in clients and restart on refresh. Do not claim snapshot isolation. Batch enrichment must not issue one taxonomy query per row.

## Native workspace

A navigation destination owns its query, selection, and presentation. List and grid render the same result state. Details preserve browser state when dismissed. The first release exposes only destinations with working data sources. Keep upload, scan, and sign-out accessible. Use shared design tokens.

## Review and compatibility

Persist human review separately from OCR and embedding state. New imports remain unreviewed even when automatically classified. Backfill legacy Filed records with migration provenance, not a fictional human actor. Processing retries preserve review decisions.

Deploy additive server changes first. Update both OpenAPI JSON copies together and compile the generated Swift client. Gate client features by an explicit supported server contract; 404 alone cannot distinguish incompatibility from denied access.

## Delivery

Each small PR owns its tests and updates the corresponding task. Do not archive this change until the implemented requirements are complete. Keep unrelated local layout edits outside the contract PR.
