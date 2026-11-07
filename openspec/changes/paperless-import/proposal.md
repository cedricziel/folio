## Why

Folio has no way to bring in an existing Paperless-ngx library. The upload API accepts only a file and a title, the document creation date is always the import time, and re-uploading the same file creates a duplicate. A library of thousands of curated documents can't be moved without losing its tags, correspondents, types, dates, custom fields, notes and OCR text, and a crash halfway through leaves a mess that can't be safely re-run.

## What Changes

- Add an import run that reads a Paperless-ngx `document_exporter` directory (v2.20 and v3.x) from a server-side path and imports it into one vault as a background job.
- Make every import idempotent: each imported document records its Paperless source and ID, unique per vault, so a re-run or a retry updates nothing it has already finished and never creates a duplicate.
- Preserve curated data: title, original file, OCR text (OCR is skipped), document date (`created`), added date (`added` becomes the document's creation time), correspondent, document type, tags (flattened, with colors), custom fields (mapped to typed metadata fields), archive serial number (as a unique integer metadata field) and notes (with their original dates).
- Verify each original file against the checksum in the export before the document becomes visible.
- Record a per-document outcome (imported, skipped, failed, with a reason) and run-level progress, publish progress on the SSE stream, and let an admin list failures and retry them.
- Offer a dry run that reports what would be imported, mapped and dropped without writing documents.
- Imported documents don't trigger `document.uploaded` workflows.
- Add JSON API routes under `/api/vaults/{vault_id}/imports` (JSON only, no HTML pages in this change), restricted to owners of that vault.
- Add a `config.toml` setting `import.allowed_roots`: the server directories an import may read from. It is a deployment setting, not in the settings table. With no roots configured, imports are disabled. A second setting, `import.max_file_bytes` (default 512 MB), caps the size of a single original.

Out of scope: saved views, workflows, mail accounts and rules, users, groups and per-document permissions, storage paths, matching rules, audit history, share links, and REST-API-based extraction from a live Paperless instance.

## Capabilities

### New Capabilities

- `paperless-import`: Import a Paperless-ngx export into a vault as a resumable, idempotent background run with per-document outcomes, progress, dry run and retry.

### Modified Capabilities

None. Imported documents go through the existing processing pipeline (thumbnails, embeddings) unchanged. OCR is skipped through the pipeline's existing rule that a document that already has text isn't re-OCR'd.

## Impact

- **folio-core:** new models for import runs and items, a Paperless manifest reader port, and an import service. Extends `CreateDocument` with an external source reference, a document date, a creation timestamp, a content checksum and a flag that suppresses workflow triggers. New repository ports for runs and items.
- **folio-infra:** SQLite and PostgreSQL repositories for runs and items, a filesystem reader for export directories, a new `JobType::ImportRun` handled by the worker, and the `import` section in `Config`.
- **folio-api:** JSON routes under `/api/vaults/{vault_id}/imports`: create (POST), list (GET), get one (`GET …/{import_id}`), list items (`GET …/{import_id}/items`), retry (`POST …/{import_id}/retry`) and cancel (`POST …/{import_id}/cancel`), plus OpenAPI schemas and the regenerated Swift client. No HTML (Turbo) routes.
- **Database:** new migration `20261107120000_paperless_import.sql` in both `migrations/sqlite` and `migrations/postgres`. It adds the nullable columns `external_source`, `external_id` and `content_sha256` to `documents`, with a unique index on `(vault_id, external_source, external_id)` where both are set, and creates the tables `import_runs` and `import_items`. Existing documents keep NULL in the new columns, and nothing is backfilled.
- **Depends on** `typed-document-metadata` persistence (tables and repositories), which is already merged. It doesn't need that change's HTTP API.
- **Security:** the export directory contains password hashes and, without `--passphrase`, plaintext mail credentials. The importer reads only the document-related records, and reads only under `import.allowed_roots`.
