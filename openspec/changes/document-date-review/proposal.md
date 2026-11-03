## Why

Document dates currently share the import timestamp, and Inbox derives review from OCR and classification. A classified document can disappear before anyone checks it. Preserve import history while allowing users to date and review documents explicitly.

## What changes

- Add optional date-only `document_date` and explicit review state, with server-derived reviewer attribution.
- Preserve legacy Filed membership with migration provenance and a null reviewer. New imports start unreviewed.
- Add date editing, review/unreview actions, and review/date filters to web and Apple detail and browse views.
- Keep processing progress and failures visible independently of review.

- Publish committed date/review transitions through the existing durable outbox and expose their workflow triggers in web and Apple.

## Capabilities

### New capabilities
- `document-review`: calendar dates, explicit review decisions, and matching web and Apple controls.

## Impact

Touches folio-core models and repository ports, SQLite and PostgreSQL adapters, folio-api JSON and HTML routes/templates, MCP metadata, OpenAPI, and the generated Swift client. Add `20261102161000_document_date.sql` to both migration directories after the lifecycle foundation is merged. Date storage lands first; review storage follows in 20261102161030_document_review.sql. Existing dates remain null. Eligible legacy Filed records receive migration provenance, without inventing a human actor. No settings change.

Depends on lifecycle foundation chat `01a19e17-80c0-7302-85f5-4eff22bc5608`. Integrate shared schema/API changes only after its PRs merge. This change implements the explicit review milestone already approved in `document-workspace`.
