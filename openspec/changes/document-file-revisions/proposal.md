## Why

A document currently identifies one mutable storage location. Replacement must preserve previous bytes, keep links stable, and prevent stale processing work from replacing the current document's content.

## What changes

- Keep logical document IDs stable and add immutable numbered file revisions with durable file identity.
- Cover web, JSON, MCP, folder, email, and scanner upload paths through the shared importer.
- Add replacement, history, historical downloads, and restore-as-new-revision in web and Apple detail views.
- Associate OCR, chunks, previews, and retained archival derivatives with their source revision.
- Define concurrent replacement, safe legacy backfill, trash/purge, and vault move behavior.

- Publish committed revision transitions through the existing durable outbox and expose their workflow triggers in web and Apple.

## Capabilities

### New capabilities
- `document-file-history`: immutable uploaded files, revision history, replacement, and restoration.

## Impact

Touches folio-core models/ports/importer, folio-infra storage/database/queue/search adapters, folio-api HTML and JSON routes, MCP resources, OpenAPI, and Apple. Add `20261102161100_document_file_revisions.sql` in both migration directories after date/review and lifecycle foundations merge. Backfill each stored legacy original as revision 1 without rewriting bytes; compute identities through a resumable verifier. No settings change. Never edit old migrations.
