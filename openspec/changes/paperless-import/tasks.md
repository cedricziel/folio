## 1. Schema and document model

- [ ] 1.1 Add migration `20261107120000_paperless_import.sql` to `migrations/sqlite` and `migrations/postgres` with every table and column in design §9:
  - the `documents` external reference and its index
  - `document_entries.external_id`
  - `import_runs`, `import_items` and `import_staging`
  - `paperless_documents`, `paperless_taxonomy` and `paperless_sync_state`
  - `paperless` in the `ingest_outcomes` and `source_health` CHECKs

  Failing test first, on SQLite and on testcontainers Postgres:
  - a second document with the same external reference is rejected even when the first is trashed
  - a row with only `external_id` set is rejected
  - `source_health` accepts `paperless`
- [ ] 1.2 Add `SourceKind::Import`, `ExternalRef`, `ImportedDocument`, `ImportedNote` and `DocumentRepository::create_imported` (SQLite and Postgres). It writes the following in one transaction:
  - the caller-supplied ID and dates
  - `ocr_status = done`, and the embed stage pending or skipped
  - the review state, tags, metadata values, and notes with `external_id` and their own `created_at`
  - the ledger row
  - the `created` outbox event with `EventOrigin::Import`

  Failing test first: read back every field, including the original `created_at`, a note's original date, the ledger row and status `filed`. A second call returns `Conflict` and leaves one row.
- [ ] 1.3 Add `DocumentRepository::apply_mirror_update`, and trash and restore with a ledger update, all with `EventOrigin::Import`. Failing test first:
  - an update replaces the title, tags and values, upserts notes by `external_id`, soft-deletes a removed imported note, keeps a Folio-native note, and advances the ledger fingerprint
  - trash and restore update `trashed_in_paperless`
- [ ] 1.4 Add `SourceKind::Import` to the OpenAPI enum and regenerate `openapi/openapi.json`. Failing test first: `tests/openapi.rs` fails until the spec includes `import`.
- [ ] 1.5 Make the workflow engine skip `EventOrigin::Import`, and the dispatcher skip `document.processed` for documents whose source is `import`. Failing tests first: the four scenarios in the workflow-triggers delta.

## 2. Run and ledger storage

- [ ] 2.1 Add the `ImportRun` and `ImportItem` models (with kind, trigger, source, action and status enums), and the ports `ImportRunRepository`, `ImportItemRepository` and `PaperlessLedgerRepository` (ledger, taxonomy map, sync state) in `crates/folio-core/src/ports/import_runs.rs`, with SQLite and Postgres adapters:
  - `insert_items_ignore_existing`
  - `claim_next`, with the conditional claim, a 10-minute stale reclaim and `attempts + 1`
  - `finish_item`, conditional on the claim
  - `counts`, computed with `GROUP BY`
  - the one-active-run rule
  - the run status transitions, ledger reads by Paperless IDs, taxonomy map upsert, and the sync-state cursor

  Failing test first:
  - inserting the same item twice keeps one row
  - two concurrent claimers never get the same item
  - a second active run in a vault returns `Conflict`
  - the cursor round-trips

## 3. Readers

- [ ] 3.1 Add the `PaperlessReader` port and the streaming export adapter `paperless_fs`, with fixtures for 2.20 (MD5) and 3.x (SHA-256, versions, a trashed document, a deleted note, an inbox tag, nested tags) under `crates/folio-infra/tests/fixtures/paperless/`. Failing test first: the expected records and skipped-model counts, and a password hash in a user record never appears in any yielded record.
- [ ] 3.2 Add the root guard. Failing test first: rejects `..` escapes, symlinks outside the root, `/imports-x` for root `/imports`, and missing paths, all with the same error.
- [ ] 3.3 Add the original-file guard. Failing test first: `../etc/passwd`, `/etc/passwd` and a symlinked original each yield `unsafe_path` without being opened.
- [ ] 3.4 Add the REST adapter `paperless_http`:
  - token auth and API version 9
  - paginated taxonomy and documents
  - `modified_since`, `list_document_ids` and the trash listing
  - metadata (checksum and size) and the original download
  - backoff on 429 and 5xx
  - the token never appears in logs or errors

  Failing test first, against a `wiremock` fake Paperless built from recorded 2.20 and 3.x JSON:
  - it yields the same records as the export fixtures for the same library
  - a 429 followed by a 200 succeeds
  - a 401 surfaces as `unauthorized`
- [ ] 3.5 Run a contract test against a real Paperless:
  - Start `ghcr.io/paperless-ngx/paperless-ngx` (pinned 2.20.x and 3.4.x tags) with Redis through testcontainers.
  - Seed a superuser, a token and a few documents with tags, custom fields and notes through its API.
  - Assert that the REST adapter reads them back with correct checksums.

  Mark the test `#[ignore]` unless `FOLIO_PAPERLESS_IT=1`, and run it in CI in a separate job.

## 4. Paperless source

- [ ] 4.1 Add `SourceConfig::Paperless` (`url`, `token`, `token_file`, `sync_mode`, `poll_interval_secs`, `default_currency`), with validation (http(s) URL, interval of at least 60 s, token or token file), token redaction (`has_token`), and `[ingest.paperless.<name>]` config parsing with its `SETTINGS` and `docs/configuration.md` entries. Failing test first: the validation errors, the redacted serialization, and the config-file parsing with the drift test.
- [ ] 4.2 Add the `/api/sources` support for `paperless`:
  - the admin guard on create, update and delete
  - 409 for a second Paperless source in a vault
  - `/test` returning the version and count, or `unreachable`, `unauthorized` or `unsupported_version`
  - the first run queued on create
  - the Paperless variant of the HTML source form

  Failing tests first: the status codes in the "Admins connect a Paperless library" requirement, run against the wiremock Paperless.
- [ ] 4.3 Register the Paperless source with `IngestScheduler`:
  - `poll()` computes the difference against the ledger
  - it creates a run only when there is a difference and no run is active
  - the cursor (with 60 s overlap) advances when a run completes
  - the daily full reconcile with fingerprints
  - modes `once`, `new_only` and `mirror`

  Failing test first:
  - an empty difference creates no run and records healthy
  - a new document creates a `create` run
  - in `mirror`, an edit, a deletion and a restore produce `update`, `trash` and `restore` items, and `new_only` produces none of them
  - a note added without a `modified` bump is found by the reconcile

## 5. Plan phase

- [ ] 5.1 Validate field kinds before any write. Failing test first: a monetary `Amount` against a text `Amount` fails the run, and the vault's taxonomy is unchanged.
- [ ] 5.2 Map taxonomy through `paperless_taxonomy`: flatten tags, rename collisions in ID order, exclude inbox tags, and rename in `mirror` mode with the collision guard. Failing test first: two plans produce the same names, a re-plan creates nothing new, and a Paperless rename renames the Folio tag.
- [ ] 5.3 Build item payloads through `import_staging`: deterministic IDs and the fingerprint. Trashed documents with no ledger row become `skipped/trashed`, and unchanged documents become `skipped/unchanged`. Failing test first: planning twice yields identical payloads and IDs, a deleted note is absent, and re-planning a completed export yields only `skipped/unchanged`.
- [ ] 5.4 Pre-validate values and drop invalid ones with warnings. Failing test first: an `ftp://` URL, a 20 KB note and a duplicate ASN each produce a warning, and the document is still applied.

## 6. Apply phase and driver

- [ ] 6.1 Apply `create`: verify, put, `create_imported`, then enqueue thumbnail and embedding jobs. Failing test first: the fully tagged fixture document matches every field, status is `filed`, and there is no OCR job.
- [ ] 6.2 Apply `update`, `trash` and `restore`, with file replacement through the staging key. Failing test first:
  - a replaced original with a bad checksum leaves the old file in place
  - a good one is replaced and re-thumbnailed
  - trash and restore round-trip
- [ ] 6.3 Fail items on checksum mismatch, missing files (including a REST 404) and oversized files. Failing test first: each reason, with the run continuing.
- [ ] 6.4 Clean up orphans. Failing test first: a permanent failure after `put` on a create leaves no object.
- [ ] 6.5 Make applying crash-safe. Failing test first: a crash injected between `put` and the transaction, then a resume, creates exactly one document and one object, and the ledger matches the document.
- [ ] 6.6 Stop crash loops. Failing test first: an item already at three attempts is marked `attempts_exhausted` without being opened.
- [ ] 6.7 Resolve document links through the ledger. Failing test first: a link to a document imported by an earlier run resolves, extra links become warnings, and unknown IDs become warnings.
- [ ] 6.8 Add `JobType::ImportRun` and its handler:
  - It runs under `with_actor(run.created_by)` and re-checks the root guard for export runs.
  - A batch stops at 100 items, 60 s, or one file over 64 MB.
  - The continuation's `scheduled_at` is the batch start.
  - Cancel is checked before each item, and finishing is idempotent.

  Failing test first: 250 items finish across three driver jobs, and each continuation is dequeued before its thumbnails.

## 7. Recovery, retry, cancel, dry run

- [ ] 7.1 Add the hook where the job gives up, which marks the run `failed`. Failing test first: a driver that always fails leaves the run `failed`, and a new run can start.
- [ ] 7.2 Add the repair step for a `running` run with no job: re-enqueue up to 3 times, then `failed`. Failing test first: delete the job row, run repair, and check the re-enqueue, then the fourth attempt fails the run.
- [ ] 7.3 Retry. Failing test first: only failed items are re-processed, and a run that failed during planning plans again.
- [ ] 7.4 Cancel. Failing test first: a `queued` run and a run waiting on backoff both end `cancelled`, and a mid-batch cancel stops after the current item.
- [ ] 7.5 Make concurrent drivers safe. Failing test first: two drivers process each item once, and the counts equal the items.
- [ ] 7.6 Dry run for both kinds: plan without writing taxonomy, verify the files, and produce the `would_*` statuses. Failing test first: a corrupted file is `failed`, the others are `would_create`, and the vault, the storage and the ledger are unchanged.

## 8. API and MCP

- [ ] 8.1 Add the `[import]` config section (`allowed_roots`, `max_file_bytes`) with env overrides, `SETTINGS` entries and docs. Failing test first: the env override parses into the list, an empty list disables export imports, and the drift test passes.
- [ ] 8.2 Add the JSON routes under `/api/vaults/{vault_id}/imports` for both kinds, with the guard, the utoipa schemas and the regenerated `openapi/openapi.json` in the same commit, then regenerate the Swift client. Failing tests first: one per spec scenario (202, 400 with identical bodies, 401, 403 `imports_disabled` and `forbidden`, 404, 409, 422).
- [ ] 8.3 Add the MCP tools `list_imports`, `get_import`, `list_import_items`, `start_import`, `retry_import` and `cancel_import`, with read and write scopes and the shared service permission check. Failing tests first: the four MCP scenarios in the spec.

## 9. Docs and verification

- [ ] 9.1 Write the admin guide in `docs/`: connecting Paperless, the sync modes and what `mirror` overwrites, exporting and mounting for large libraries, dry runs, import, retry and cancel, MCP usage, and deleting exports. Delegate the writing to the technical-writer agent.
- [ ] 9.2 Verify end to end with the `verify` skill:
  - Start an ephemeral Folio and a testcontainers Paperless.
  - Connect them in `mirror` mode and import.
  - Kill Folio mid-run and resume.
  - Edit, delete and restore in Paperless, then check that the next sync mirrors those changes.
  - Re-run an export import of the same library and check that it changes nothing.
  - Confirm there are no duplicates and no orphaned objects.
- [ ] 9.3 Run `make format`, `make lint` and `make test`.
