## Why

Folio has no way to bring in an existing Paperless-ngx library. The upload API accepts only a file and a title, the document creation date is always the import time, and re-uploading the same file creates a duplicate. A library of thousands of curated documents can't be moved without losing its tags, correspondents, types, dates, custom fields, notes and OCR text, and a crash halfway through leaves a mess that can't be safely re-run.

## What Changes

- Add an import run that reads a Paperless-ngx `document_exporter` directory (2.16 or later, including 3.x) from a server-side path and imports it into one vault as a background job.
- Make every import idempotent:
  - Each imported document records its Paperless source and ID, unique per vault.
  - The Folio document ID is derived from that reference, so a re-run, a resumed run or a retry never creates a duplicate and never leaves orphaned files.
- Make every run finish: a run whose background job dies or gives up is recovered or marked failed, so it can't lock the vault.
- Preserve curated data:
  - title, original file, OCR text (OCR is skipped), document date (`created`) and added date (`added` becomes the document's creation time)
  - correspondent, document type, tags (flattened, with colors)
  - custom fields (mapped to typed metadata fields), archive serial number (as a unique integer metadata field) and notes (with their original dates)
  - Documents with the Paperless inbox tag need review; all others are imported as reviewed.
  - A value Folio can't hold is dropped with a warning on the item instead of failing the whole document.
- Verify each original file against the checksum in the export before the document is written. Reject file paths in the manifest that point outside the export directory.
- Record a per-document outcome (imported, skipped, failed, with a reason and warnings) and run-level counts, and let the admin list failures, retry them, or cancel a run. Clients follow progress by reading the run; this change adds no SSE event types.
- Offer a dry run that reads every file and reports what would be imported, mapped, dropped or rejected, without writing anything.
- Imported documents don't start `document.uploaded` or `document.processed` workflows.
- Add JSON API routes under `/api/vaults/{vault_id}/imports` (JSON only, no HTML pages in this change). Only an instance admin who also owns the target vault can use them, because an import reads server files.
- Add a `config.toml` section `[import]`. Paths are a deployment decision, so these settings aren't in the settings table, and the `config.toml` value or its `FOLIO_IMPORT__*` env override is the only source.
  - `allowed_roots`: the server directories an import may read from. With no roots configured, imports are disabled.
  - `max_file_bytes`: the size limit for one original. The default is 512 MB.
- Add `import` as a document source kind in the JSON API (`SourceKind`).

Out of scope:
- saved views, workflows, mail accounts and rules, users, groups and per-document permissions
- storage paths, matching rules, audit history and share links
- older Paperless 3 file versions
- Paperless's archive PDFs (originals only)
- duplicate-content detection against documents that were uploaded by hand
- REST-API-based extraction from a live Paperless instance

## Capabilities

### New Capabilities

- `paperless-import`: Import a Paperless-ngx export into a vault as a resumable, idempotent background run with per-document outcomes, dry run, retry and cancel.

### Modified Capabilities

- `workflow-triggers`: Documents whose source is `import` don't start `document.uploaded` or `document.processed` workflows.

## Impact

- **folio-core:**
  - New models for import runs and items, a Paperless export reader port, and an import service.
  - A new `ImportedDocument` input and `DocumentRepository::create_imported`, which writes a document with a caller-supplied ID, dates, external reference, checksum, review state, tags, metadata values and notes in one transaction.
  - New repository ports for runs and items, and `SourceKind::Import`.
- **folio-infra:**
  - SQLite and PostgreSQL repositories for runs and items, and a filesystem reader for export directories.
  - A new `JobType::ImportRun` handled by the worker, and a repair-loop step for stalled runs.
  - Workflow trigger suppression for imported documents.
  - The `[import]` section in `Config`, its `SETTINGS` entries and `docs/configuration.md`.
- **folio-api:** JSON routes under `/api/vaults/{vault_id}/imports`:
  - create (`POST`) and list (`GET`)
  - get one (`GET …/{import_id}`) and list its items (`GET …/{import_id}/items`)
  - retry (`POST …/{import_id}/retry`) and cancel (`POST …/{import_id}/cancel`)
  - OpenAPI schemas and the regenerated Swift client. No HTML (Turbo) routes.
- **Database:** new migration `20261107120000_paperless_import.sql` in both `migrations/sqlite` and `migrations/postgres`.
  - It adds the nullable columns `external_source`, `external_id` and `content_sha256` to `documents`. A CHECK makes `external_source` and `external_id` set together, with a unique index on `(vault_id, external_source, external_id)` where both are set.
  - It creates the tables `import_runs` and `import_items`.
  - Existing documents keep NULL in the new columns, and nothing is backfilled.
- **Depends on** `typed-document-metadata` persistence (tables and repositories), which is already merged. It doesn't need that change's HTTP API.
- **Security:** the export directory contains password hashes and, without `--passphrase`, plaintext mail credentials. The importer reads only document-related records, only under `import.allowed_roots`, and only for instance admins.
