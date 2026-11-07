## 1. Schema and document model

- [ ] 1.1 Add migration `20261107120000_paperless_import.sql` to `migrations/sqlite` and `migrations/postgres`. It adds the `documents.external_source`, `external_id` and `content_sha256` columns, the paired-NULL CHECK, the `documents_external_ref` partial unique index, and the `import_runs`, `import_items` and `import_staging` tables. Failing test first: on both backends, a second document with the same `(vault_id, external_source, external_id)` is rejected even when the first one is trashed, and a row with only `external_id` set is rejected.
- [ ] 1.2 Add `SourceKind::Import`, `ExternalRef`, `ImportedDocument` and `ImportedNote` to `folio-core`, plus `DocumentRepository::create_imported`. Implement it for SQLite and PostgreSQL. In one transaction it writes:
  - the caller-supplied `id`, `created_at`, `updated_at`, `document_date` and `content_sha256`
  - `ocr_status = done`, and the embed stage pending (or skipped when there is no text)
  - the review state, tags and metadata values
  - notes with their own `created_at`
  - the `created` outbox event

  Failing test first: read back every field, including the original `created_at` and a note's original date, with status `filed`. A second call with the same external ref returns `Conflict` and leaves one row.
- [ ] 1.3 Add `SourceKind::Import` to the OpenAPI `SourceKind` enum and regenerate `openapi/openapi.json`. Failing test first: `tests/openapi.rs` fails until the spec includes `import`.
- [ ] 1.4 Make workflow dispatch skip `document.uploaded` and `document.processed` occurrences whose source is `import`. Failing tests first: enabled workflows on those two events produce no run for an imported document, and a `document.tags_changed` workflow still runs when a tag is added to it later.

## 2. Import run storage

- [ ] 2.1 Add `ImportRun`, `ImportItem`, their status enums, and the `ImportRunRepository` and `ImportItemRepository` ports in `folio-core`. Implement them for SQLite and PostgreSQL:
  - `insert_items_ignore_existing`
  - `claim_next(run, claim)`, the conditional claim with the 10-minute stale reclaim that increments `attempts`
  - `finish_item(claim, status, reason, warnings)`, conditional on the claim
  - `counts(run)`, computed with `GROUP BY`
  - the one-active-run-per-vault constraint

  Failing test first: inserting the same item twice keeps one row, two concurrent claimers never get the same item, and a second `running` run in a vault returns `Conflict`.
- [ ] 2.2 Mark already-imported items with one query that joins items to `documents` on the external ref. Failing test first: after one document with external ID `7` exists, planning an item with ID `7` marks it `skipped` with reason `already_imported`.

## 3. Export reader and path safety

- [ ] 3.1 Add the `PaperlessExportReader` port and the streaming `paperless_fs` adapter, with fixtures for 2.20 (MD5) and 3.x (SHA-256, file versions, a trashed document, a deleted note, an inbox tag, nested tags) under `crates/folio-infra/tests/fixtures/paperless/`. Failing test first: the reader yields the expected typed records and skipped-model counts, and a fixture with a user record containing a password hash never produces that hash in any yielded record.
- [ ] 3.2 Add the root guard: canonicalize the path, then require a component-wise prefix match against `import.allowed_roots`. Failing test first: rejects `..` escapes, symlinks pointing outside the root, a sibling `/imports-x` for root `/imports`, and missing paths, all with the same error.
- [ ] 3.3 Add the original-file guard: resolve each manifest path under the canonical export directory, and refuse absolute paths, `..` components and symlinks on any component. Failing test first: manifest entries `../etc/passwd`, `/etc/passwd`, and a symlinked original each yield `unsafe_path`, and the file is never opened.

## 4. Plan phase

- [ ] 4.1 Validate custom field kinds against the vault before any write. Failing test first: an export whose `Amount` field is monetary, imported into a vault where `Amount` is text, ends `failed` with both kinds named, and the vault's tag, correspondent, type and field counts are unchanged.
- [ ] 4.2 Resolve taxonomy:
  - tags flattened, with `<parent>/<name>` renames in Paperless ID order
  - inbox tags excluded
  - correspondents and types by name
  - fields and select options by name and label
  - the `ASN` field

  Failing test first: two runs over the nested-tag fixture produce the same names, and re-planning creates no second tag.
- [ ] 4.3 Build item payloads through `import_staging`. Each payload holds the deterministic UUIDv5 document ID, the mapped IDs and values, the notes minus deleted ones, the raw links and the version count. Trashed documents get `skipped`/`trashed`, and staging is dropped at the end. Failing test first: planning twice yields identical payloads and IDs, and the deleted note is absent.
- [ ] 4.4 Pre-validate values and drop invalid ones with warnings: URL scheme, text and long-text length, note body, and an ASN that collides with an existing value. Failing test first: an `ftp://` URL, a 20 KB note and a duplicate ASN each produce a warning, and the document still imports.

## 5. Import phase

- [ ] 5.1 Import one item: claim it, check size, hash while reading, verify the checksum, put the file to `documents/{id}/original`, call `create_imported`, then enqueue thumbnail and embedding jobs. Failing test first: the fully tagged fixture document matches title, dates, taxonomy, metadata, ASN, content, note, creator and status, and the queue holds no `ExtractText` or `ProcessDocument` job for it.
- [ ] 5.2 Handle checksum and missing-file failures. Failing test first: a corrupted original ends `failed`/`checksum_mismatch`, a missing one ends `file_missing`, no document is created, and the run continues.
- [ ] 5.3 Clean up orphans. Failing test first: a fault-injecting store that fails `create_imported` permanently after `put` leaves no object at that key once the item ends `failed`.
- [ ] 5.4 Make the import crash-safe and idempotent. Failing test first: a crash injected between `put` and the transaction, followed by a resume, creates exactly one document and one object. A second run over the same export creates no documents and calls `put` zero times.
- [ ] 5.5 Enforce the file size limit. Failing test first: a file over `max_file_bytes` ends `file_too_large`.
- [ ] 5.6 Stop crash loops. Failing test first: an item already at three attempts is marked `attempts_exhausted` at claim time without being opened.
- [ ] 5.7 Resolve document links in the final pass, through external refs across runs. Failing test first: a link to a document imported by an earlier run resolves, a second link in the same value becomes a warning, and a link to an unknown ID becomes a warning.
- [ ] 5.8 Add `JobType::ImportRun { run_id }` and its handler:
  - It runs under `with_actor(run.created_by)` and checks the root guard again at start.
  - A batch ends at 100 items, 60 s, or after one file over 64 MB.
  - It enqueues the continuation with `scheduled_at` set to the batch start, checks for cancel before each item, and finishes idempotently.

  Failing test first: a run over 250 fixture items completes across three driver jobs, and each continuation is dequeued before the thumbnail jobs its batch queued.

## 6. Recovery, retry and cancel

- [ ] 6.1 Add a hook where an `ImportRun` job becomes `failed` after its last attempt, which marks the run `failed` with the job's error. Failing test first: a driver that fails every attempt (export directory removed) leaves the run `failed`, and a new run in the vault can be created.
- [ ] 6.2 Add a repair-loop step that re-enqueues a driver for a `running` run with no active `ImportRun` job, up to 3 times, then marks the run `failed`. Failing test first: deleting the run's job row and running repair re-enqueues it, and the fourth time marks the run `failed`.
- [ ] 6.3 Retry: reset failed items to `pending` with zero attempts, restart the plan phase when `plan_complete` is false, and enqueue a driver. Failing test first: a run with two failed and three imported items re-processes only the two, and a run that failed during planning plans again.
- [ ] 6.4 Cancel: set `cancel_requested`, and move straight to `cancelled` when no driver job is active. Failing test first: cancelling a `queued` run and a run whose job is waiting on backoff both end `cancelled`, and cancelling mid-batch stops after the current item.
- [ ] 6.5 Make concurrent drivers safe. Failing test first: two drivers started for the same run finish with each item processed once, and counts equal the items grouped by status.

## 7. Dry run

- [ ] 7.1 Make a dry run plan without creating taxonomy, then open and verify every file without writing: items end `would_import`, `skipped` or `failed`. Failing test first: after a dry run over a fixture with one corrupted file, that item is `failed`/`checksum_mismatch`, the others are `would_import`, and the vault's documents, taxonomy, fields and storage are unchanged.

## 8. API and config

- [ ] 8.1 Add the `[import]` config section (`allowed_roots`, `max_file_bytes`) with `FOLIO_IMPORT__*` overrides, `SETTINGS` entries and `docs/configuration.md`. Failing test first: the env override parses into the list, an empty list disables imports, and the settings drift test passes only once the docs list both keys.
- [ ] 8.2 Add the JSON routes under `/api/vaults/{vault_id}/imports` (create, list, get, items, retry, cancel) with the admin-plus-owner guard, their utoipa schemas and the regenerated `openapi/openapi.json` in the same commit, then regenerate the Swift client. Failing tests first: one per spec scenario, covering 202, 400 (outside root and missing path giving identical bodies), 401, 403 `imports_disabled`, 403 `forbidden` for a vault owner who isn't an admin, 404, 409 and 422.

## 9. Docs and verification

- [ ] 9.1 Write the admin guide in `docs/`: how to export from Paperless (`document_exporter` with `--passphrase`), mount the export read-only, dry run, import, retry, cancel, and delete the export afterwards. Delegate the writing to the technical-writer agent.
- [ ] 9.2 Verify end to end with the `verify` skill:
  - Start an ephemeral instance with a fixture export mounted.
  - Do a dry run, then import, kill the server mid-run, restart, and retry.
  - Confirm the counts, that there are no duplicates, and that no storage objects are orphaned.
- [ ] 9.3 Run `make format`, `make lint` and `make test`.
