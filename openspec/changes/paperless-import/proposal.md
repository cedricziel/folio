## Why

Folio has no way to bring in an existing Paperless-ngx library or to keep using Paperless while moving over:
- The upload API accepts only a file and a title.
- The document creation date is always the import time.
- Re-uploading the same file creates a duplicate.

A library of thousands of curated documents can't be moved without losing its tags, correspondents, types, dates, custom fields, notes and OCR text. A crash halfway through leaves a mess that can't be safely re-run. Nothing keeps Folio current while Paperless is still the place where scans land.

## What Changes

- **Paperless connection as a source.** A vault gets a Paperless connection as a new ingestion source kind, `paperless`, next to folder and email.
  - It holds the Paperless URL and an API token.
  - It is configured on the vault's Sources page and through the sources API, or as `[ingest.paperless.<name>]` in `config.toml`.
  - It has a per-source **sync mode**:
    - `once`: a single full import, then no polling.
    - `new_only`: import documents that are new in Paperless.
    - `mirror` (the default): Paperless is the master. New documents are imported, and Paperless edits, file changes, deletions and restores are applied to Folio, overwriting changes made in Folio.
  - It has a per-source **poll interval** (default 15 minutes) and a daily full reconcile.
- **Export-folder import** for very large libraries or offline moves. An import reads a Paperless `document_exporter` directory from a server path under `import.allowed_roots`. Export and connection share the vault's Paperless library identity, so a bulk export import followed by a connection continues seamlessly.
- **One engine for both.**
  - Every first import and every sync cycle that finds changes is an **import run**: a background job with per-document items, a create/update/trash/restore action, outcomes, retry and cancel.
  - A vault holds at most one Paperless library.
  - A per-document sync ledger records which Paperless document maps to which Folio document, with the last-synced Paperless state. This is how "already done" and "still to do" are known.
- **Idempotent and crash-safe.**
  - Folio document IDs are derived from the vault and the Paperless ID, so a re-run, resume or retry never duplicates and never orphans files.
  - Runs always reach a final status, and concurrent drivers are safe.
- **Curated data is preserved and mirrored:**
  - title, original file, OCR text (OCR is skipped)
  - document date, added date, correspondent, document type, tags (flattened, with colors)
  - custom fields (as typed metadata fields), archive serial number (as a unique integer field) and notes
  - The inbox tag maps to "needs review".
  - A value Folio can't hold is dropped with a warning.
- **Checks on what is read.**
  - Each original is verified against its Paperless checksum.
  - Manifest file paths can't escape the export directory.
- **Dry runs** report what would change, without writing.
- **Workflows.** Changes made by an import or a sync don't start workflows, and imported documents don't start `document.processed` workflows.
- **API and MCP.**
  - JSON API routes live under `/api/vaults/{vault_id}/imports`: start an export import or a sync-now, list runs, get a run, list items, retry, cancel. The existing `/api/sources` routes accept the `paperless` kind.
  - Matching MCP tools: `list_imports`, `get_import`, `list_import_items`, `start_import`, `retry_import`, `cancel_import`.
  - Paperless sources and imports require an instance admin who also owns the vault.
- **Settings.** `[import]` in `config.toml` has `allowed_roots` and `max_file_bytes`; these are config-only, because server paths are a deployment decision. Paperless source settings follow the existing source pattern: database-managed sources from the Sources page, or config-defined sources in `config.toml`, with the same precedence as folder and email sources.

Out of scope:
- saved views, workflows, mail accounts and rules, users, groups and per-document permissions
- storage paths, matching rules, audit history and share links
- older Paperless 3 file versions and Paperless archive PDFs
- writing changes back to Paperless (sync is one-way)
- web UI pages for import runs (designed on the Folio Screens canvas; built in a follow-up change)

## Capabilities

### New Capabilities

- `paperless-import`: Import and continuously sync a Paperless-ngx library into a vault, from a connection or an export folder, as resumable, idempotent runs with a sync ledger, sync modes, dry run, retry, cancel, a JSON API and MCP tools.

### Modified Capabilities

- `workflow-triggers`: Events produced by imports and syncs don't start workflows. Imported documents don't start `document.processed` workflows.

## Impact

- **folio-core:**
  - Models for import runs, items, the sync ledger and the taxonomy map.
  - A source-agnostic `PaperlessReader` port with two adapters (export folder, REST).
  - An import service with plan and apply phases.
  - `ImportedDocument` and `DocumentRepository::create_imported`, plus `apply_mirror_update`, trash and restore.
  - `SourceKind::Import` (document origin), `SourceKind::Paperless` (source kind), `SourceConfig::Paperless`, and `EventOrigin::Import`.
- **folio-infra:**
  - The export-folder and REST (`reqwest`) Paperless readers.
  - SQLite and PostgreSQL repositories for runs, items, ledger, taxonomy map and sync state.
  - `JobType::ImportRun` in its own `JobClass::Import` worker lane, so imports don't block processing of user uploads, and the stalled-run repair step.
  - Paperless source polling through the existing `IngestScheduler`.
  - The `[import]` and `[ingest.paperless.*]` config sections with their `SETTINGS` entries.
- **folio-api:**
  - JSON routes under `/api/vaults/{vault_id}/imports`, and the `paperless` kind on `/api/sources`, including `/test`.
  - The existing HTML source form gains a Paperless variant (URL, token, sync mode, interval). No new HTML pages for runs.
  - OpenAPI schemas and the regenerated Swift client.
- **folio-mcp:** six import tools, with write scope for the mutating ones.
- **Database:** new migrations, one per implementation PR, named `202611071xxxxx_paperless_*.sql`, each in both `migrations/sqlite` and `migrations/postgres`. The upgrade is forward-only: the previous binary can't decode the new `import` origin and source kind, so admins back up first.
  - `documents`: `external_source`, `external_id`, `content_sha256`, with the paired-NULL CHECK and a partial unique index.
  - `document_entries`: `external_id`, so mirrored notes can be updated.
  - New tables: `import_runs`, `import_items`, `import_staging`, `paperless_documents` (the sync ledger), `paperless_taxonomy` (the taxonomy map) and `paperless_sync_state`.
  - The `source_kind` CHECKs on `ingest_outcomes` and `source_health` gain `paperless`, and a partial unique index allows one live `paperless` source per vault.
  - Existing rows keep NULL in the new columns, and nothing is backfilled.
- **Depends on** `typed-document-metadata` persistence, which is already merged.
- **Security:**
  - The API token is redacted like the email password. `token_file` is accepted only in `config.toml`. A URL change requires the token again. Redirects to another origin aren't followed.
  - Every operation on a `paperless` source, including test and restore, requires an instance admin who owns the vault.
  - A mirror never trashes from an incomplete or empty listing, pauses above a trash threshold until an admin confirms, and refuses to sync a library that doesn't match the vault's recorded one.
  - Export folders contain password hashes and possibly plaintext mail credentials, and the readers never deserialize those records.
  - Only instance admins can create Paperless sources or start imports, because both read server files or reach arbitrary URLs.
