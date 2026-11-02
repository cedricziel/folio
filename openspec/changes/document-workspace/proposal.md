# Document workspace

## Why

The Apple app puts document rows in a narrow sidebar and leaves most of the window empty until selection. It cannot browse classifications, combine search with filters, or retrieve subsequent search pages. Review completion is inferred from metadata rather than a human decision.

## What changes

Implement the sequence in `docs/document-workspace-plan.md`: a reusable document browser, explicit review, taxonomy navigation and management, private saved views, and bulk classification. Deliver the library browser first.

The initial backend increment adds paged repository queries without changing existing callers. JSON browsing uses the access policy delivered by the active vault-access change. The workspace adds read authorization for its two browse routes; vault management and migration of other routes remain separate. Preserve the existing web form routes and hybrid search while introducing the new contracts. HTML redesign is outside this change.

## Capabilities

- New `document-workspace`: bounded library browsing, navigation, review decisions, and saved workflow state.
- Later slices update document metadata and search capabilities alongside their implementations, using the approved plan as the delivery order.

## Impact

- `folio-core`: query/page models and repository port; subsequent review and saved-view models.
- `folio-infra`: matching SQLite and PostgreSQL queries and migrations.
- `folio-api`: JSON browse/edit/review APIs and existing HTML compatibility; OpenAPI schema.
- Apple: generated API client, FolioKit state, native navigation and document browser.
- Database: later migrations use the suffixes `document_review`, `document_metadata_revision`, `saved_views`, and `document_date` in both migration directories. Allocate sequence numbers when implementing. Preserve legacy Filed membership with migration provenance; existing document dates remain null.
- Preferences: saved views belong to a user and vault; view restoration is device-local. No new instance settings or config.toml values.

## Dependencies

Use the access contract from `add-vaults-and-access` before exposing explicit-vault routes. Coordinate processing and semantic search with `pluggable-processing-pipeline`. Repository pagination and native navigation can proceed without those routes.
