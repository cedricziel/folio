## Why

Documents need vault-specific fields for dates, amounts, references, and other structured information. Users must manage these fields and edit document values in both the web application and Apple app.

## What changes

- Add vault-owned definitions, stable select options, and typed document values with lifecycle timestamps and server-derived audit actors.
- Support text, long text, boolean, integer, decimal, date, money with currency, select, URL, and document links.
- Validate values before persistence, preserve exact decimal amounts, and enforce vault-local references and optional uniqueness.
- Add JSON API endpoints, HTML management and document forms, OpenAPI schemas, and the generated Swift client contract.
- Add native field management, value editing, display, and document-link navigation using the existing workspace and design tokens.
- Add bounded exact-value filtering without changing existing search behavior.

## Capabilities

### New capabilities

- `typed-document-metadata`: Manage typed fields in a vault and attach validated values to logical documents.

## Impact

Touches folio-core models and repository ports, folio-infra SQLite and PostgreSQL adapters, folio-api routes and Askama templates, OpenAPI, and Apple service/view models/views. No config.toml or settings-table setting is added.

New migrations `20261102162000_typed_document_metadata.sql` in both database directories create definitions, options, and values. Existing documents receive no values. Existing migrations remain unchanged. The slice reserves versions 20261102162000–20261102162999, using valid timestamp suffixes and identical versions in both databases. Check main for unrelated collisions before adding migrations. Foundation reserves 20261102160000–20261102160400. Fresh databases must apply prerequisites first; upgrade tests cover independent dependent-slice merge orders. Never renumber a merged or applied migration.

Foundation lifecycle and ownership changes must merge first. Logical document IDs remain `documents.id`, independently of file revision IDs. The versioning slice owns file revisions. Metadata owns neither file bytes nor revision history.
