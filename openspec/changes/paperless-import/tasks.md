## 1. Schema and document model

- [ ] 1.1 Add migration `20261107120000_paperless_import.sql` to `migrations/sqlite` and `migrations/postgres`: the `documents.external_source`, `external_id` and `content_sha256` columns, the `documents_external_ref` partial unique index, and the `import_runs` and `import_items` tables. Failing test first: a migration test inserts two documents with the same `(vault_id, external_source, external_id)` and expects a unique violation on both backends.
- [ ] 1.2 Add `SourceKind::Import`, `ExternalRef`, `ImportedDocument` and `ImportedNote` to `folio-core` models, and `DocumentRepository::create_imported`, which writes the document, tags, metadata values and notes in one transaction with caller-supplied `id`, `created_at`, `updated_at`, `document_date` and `content_sha256`. Failing test first: the repository test creates an imported document and reads back all fields, including the original `created_at`. A second call with the same external ref returns a `Conflict` error and leaves one row.
- [ ] 1.3 Make the workflow trigger evaluator ignore `document.uploaded` for `SourceKind::Import`. Failing test first: an enabled `document.uploaded` workflow produces no run for an imported document.

## 2. Import run storage

- [ ] 2.1 Add `ImportRun`, `ImportItem`, their status enums and the `ImportRunRepository`/`ImportItemRepository` ports in `folio-core`. Implement them for SQLite and PostgreSQL, including `insert_items_ignore_existing`, `next_pending_batch(run, n)`, `mark(item, status, reason, warnings)`, `counts(run)` and the one-active-run-per-vault constraint. Failing test first: inserting the same item twice keeps one row, and creating a second `running` run in one vault returns `Conflict`.

## 3. Export reader

- [ ] 3.1 Add the `PaperlessExportReader` port and the `paperless_fs` adapter. The adapter streams `manifest.json`, reads `metadata.json`, supports `--split-manifest` and `--use-folder-prefix`, and counts skipped record models without deserializing user, mail-account or token records. Failing test first: fixture exports for v2.20 (MD5) and v3.x (SHA-256, file versions, trashed document) under `crates/folio-infra/tests/fixtures/paperless/` yield the expected documents, taxonomy and skipped counts.
- [ ] 3.2 Add the path guard: canonicalize the requested path, then require a component-wise prefix match against `import.allowed_roots`. Failing test first: `..` escapes and symlinks pointing outside the root are rejected, and a sibling directory named like the root plus a suffix (`/imports-x` for root `/imports`) is rejected.

## 4. Plan phase

- [ ] 4.1 Implement taxonomy resolution in the import service: tags (flattened, `<parent>/<name>` on collision), correspondents and types by name, custom fields to metadata kinds, select options by label, and the `ASN` field. A kind conflict fails the run before any document is written. Failing test first: an export whose `Amount` field is monetary, imported into a vault where `Amount` is text, ends `failed` with an error naming both kinds and creates no documents.
- [ ] 4.2 Create the items with pre-allocated document IDs, and skip trashed documents with reason `trashed`. Failing test first: running the plan phase twice over the same export yields the same item set and the same pre-allocated IDs.
- [ ] 4.3 Dry run: the plan phase only, items marked `would_import`, and taxonomy reported without being created. Failing test first: after a dry run, the vault's tag, correspondent, type, field and document counts are unchanged.

## 5. Import phase

- [ ] 5.1 Import one item: hash while reading, verify the manifest checksum, put the file to `documents/{id}/original`, call `create_imported`, then enqueue thumbnail and embedding jobs and no OCR job. Failing test first: a fully tagged fixture document ends with matching title, dates, taxonomy, metadata, ASN, content and note, and the queue holds no `ExtractText` or `ProcessDocument` job for it.
- [ ] 5.2 Make items crash-safe and idempotent: an external-ref conflict marks the item `skipped`/`already_imported`, a checksum mismatch marks it `failed`/`checksum_mismatch`, a missing file marks it `file_missing`, an oversized file marks it `file_too_large`, and item attempts are capped at 3. Failing tests first: (a) a fault-injecting storage that fails after `put`, followed by a resume, creates exactly one document; (b) a second run over the same export creates none.
- [ ] 5.3 Resolve document links in a second pass, with warnings for targets outside the run, plus `duplicate_content` warnings by `content_sha256`. Failing test first: a link to a document in the export resolves, a link to a missing document yields a warning, and a byte-identical earlier upload yields a warning that names it.
- [ ] 5.4 Add `JobType::ImportRun { run_id }` and its worker handler: one batch of 25 per job, re-enqueue while items are pending, check cancel between items, and finish with the final status and summary. Failing test first: a run over 60 fixture items completes across three driver jobs, and a cancel requested after the first batch ends the run `cancelled` with 25 items imported.

## 6. API, events, config

- [ ] 6.1 Add the `[import]` config section (`allowed_roots`, `max_file_bytes`) with env overrides. Failing test first: `FOLIO_IMPORT__ALLOWED_ROOTS` parses into the list, and an empty list disables imports.
- [ ] 6.2 Add the JSON routes `POST/GET /api/vaults/{vault_id}/imports`, `GET …/{import_id}`, `GET …/{import_id}/items?status=`, `POST …/{import_id}/retry` and `POST …/{import_id}/cancel`, with owner-only access. Failing tests first: one per spec scenario for status codes 202, 400, 401, 403, 404, 409 and 422.
- [ ] 6.3 Add the throttled `ImportProgress` and `ImportFinished` events on `/events`. Failing test first: a subscribed client receives at most one progress event per second for a run and exactly one finished event.
- [ ] 6.4 Update `openapi/openapi.json` with the new schemas and paths, and regenerate the Swift client. Failing test first: the existing OpenAPI drift check fails until the spec is regenerated.

## 7. Docs and verification

- [ ] 7.1 Write the admin guide in `docs/` covering how to export from Paperless (`document_exporter` with `--passphrase`), mount the export, dry run, import, retry, and delete the export afterwards. Delegate the writing to the technical-writer agent.
- [ ] 7.2 Verify end to end with the `verify` skill: start an ephemeral instance with a fixture export mounted, do a dry run, import, kill and restart the worker mid-run, retry, and confirm the counts and that there are no duplicates.
- [ ] 7.3 Run `make format`, `make lint` and `make test`.
