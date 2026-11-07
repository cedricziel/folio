Migrations are split by pull request. Each PR adds its own `202611071xxxxx_paperless_*.sql` file, with the same version in `migrations/sqlite` and `migrations/postgres`, and never edits a merged one.

## 1. Document model

- [ ] 1.1 Migration for the `documents` external reference (columns, paired-NULL CHECK, partial unique index that includes trashed rows) and `document_entries.external_id` with its partial unique index. Failing test first, on SQLite and on testcontainers Postgres: a duplicate external reference is rejected even when the first document is trashed, and a row with only `external_id` set is rejected.
- [ ] 1.2 Add `SourceKind::Import`, `EventOrigin::Import`, `ExternalRef`, `ImportedDocument`, `ImportedNote` and `DocumentRepository::create_imported` (SQLite and Postgres). It writes the following in one transaction:
  - the caller-supplied ID and dates
  - `ocr_status = done`, and the embed stage pending or skipped
  - the review state, tags, metadata values, and notes with `external_id`
  - the `created` outbox event with origin `Import`

  Failing test first: read back every field, with status `filed`. A second call returns `Conflict`.
- [ ] 1.3 Add `SourceKind::Import` to the OpenAPI enum. Failing test first: `tests/openapi.rs` fails until it is there.
- [ ] 1.4 Make the workflow engine skip `EventOrigin::Import`, give processing jobs an `origin` field, and have the worker emit `Processed` with the job's origin. Failing tests first: the four workflow-triggers scenarios, including a user-started reprocess that does trigger.

## 2. Run, ledger and sync storage

- [ ] 2.1 Migration for `import_runs` (including `cursor_after` and `confirm_trash`), `import_items`, `import_staging`, `paperless_documents` (including `needs_reapply` and `folio_state`), `paperless_taxonomy` and `paperless_sync_state` (including `library_sample`, `once_done`, `retry_ids` and `parked_ids`). It also adds `paperless` to the `ingest_outcomes` and `source_health` CHECKs (a table rebuild on SQLite) and a partial unique index for one live `paperless` source per vault. Failing test first: on both backends, `source_health` accepts `paperless`, and a second live `paperless` source in a vault is rejected.
- [ ] 2.2 Add the models and the `ImportRunRepository`, `ImportItemRepository` and `PaperlessLedgerRepository` ports in `crates/folio-core/src/ports/import_runs.rs`, with SQLite and Postgres adapters:
  - the claim, finish, counts and one-active-run rule
  - the ledger, the taxonomy map and the sync state with the cursor, retry and park sets

  Failing test first:
  - two concurrent claimers never share an item
  - a second active run returns `Conflict`
  - the cursor, `retry_ids` and `library_sample` round-trip
- [ ] 2.3 Add `apply_mirror_update` (versioned file-key switch, note upsert that keeps notes deleted in Folio deleted), and trash and restore methods that update the ledger in one transaction with origin `Import`. Failing test first:
  - an update replaces the title, tags and values, keeps a Folio-native note, and doesn't resurrect a note deleted in Folio
  - a failed transaction leaves the old `file_path`
  - trash and restore update the ledger
- [ ] 2.4 Keep `folio_state` current when users trash, restore and purge documents that have a ledger row. Failing test first: a user trash sets `trashed_by_user`, and a purge sets `purged_in_folio`.

## 3. Readers

- [ ] 3.1 Add the `PaperlessReader` port and the streaming export adapter, with the 2.20 and 3.x fixtures. Failing test first: the expected records, and a password hash never appears in any record.
- [ ] 3.2 Add the root guard. Failing test first: `..`, symlinks outside the root, `/imports-x`, and missing paths, all with the same error.
- [ ] 3.3 Add the original-file guard. Failing test first: `../etc/passwd`, `/etc/passwd` and a symlinked original yield `unsafe_path` without being opened.
- [ ] 3.4 REST adapter, part 1: client with token auth, API version 9, a same-origin redirect policy and fixed error reasons, plus paginated taxonomy and document records. Failing test first, against `wiremock`: the records equal the export fixture's for the same library, and a cross-origin redirect yields `redirected` without sending the token.
- [ ] 3.5 REST adapter, part 2: keyset `modified_since`, `CompleteIdSet` listing, the trash listing, metadata and download, backoff, and the token never logged. Failing test first:
  - a page failure or a count mismatch yields an error, not a set
  - a document modified mid-listing isn't skipped
  - a 429 followed by a 200 succeeds
- [ ] 3.6 Contract test against real Paperless 2.20.x and 3.4.x in testcontainers, with Redis and seeded data. It is ignored unless `FOLIO_PAPERLESS_IT=1` and runs as its own CI job. Failing test first: the records and checksums read back match the seeded data.

## 4. Paperless source

- [ ] 4.1 Add `SourceConfig::Paperless` with validation (http(s) URL, interval of at least 60 s), `REDACTED` token handling (the placeholder is kept only when the URL is unchanged), `token_file` from config only, `[ingest.paperless.<name>]` with a required `vault`, `SETTINGS` entries and docs. Failing test first:
  - the validation errors
  - a URL change with the placeholder fails
  - config without `vault` fails at load
  - the drift test passes
- [ ] 4.2 Add the `Caller` value and the `CallerResolver` port. Implement them in folio-infra from folio-auth, with `auth.required = false` mapping to an admin. Failing test first: an admin, a non-admin owner, and auth disabled each resolve as expected.
- [ ] 4.3 Apply the admin-plus-owner guard to every `paperless` source operation, JSON and HTML: create, update, toggle, delete, restore, test and try-connection. Add 409 for a second source, with config winning. Failing tests first: 403 for each operation by a non-admin owner, with no outbound request made (wiremock records none), and 409 cases.
- [ ] 4.4 Extend `/test` with the version, the document count, `superuser` and the reason. Update `SourceTestResult` in OpenAPI and regenerate. Failing test first: each reason, and `superuser: false` for a limited token.
- [ ] 4.5 Add the Paperless variant of the HTML source form (URL, token, sync mode, interval). Failing test first: a route test renders and submits it. Verify it with the screen tour.
- [ ] 4.6 Register the Paperless source with `IngestScheduler`:
  - the first poll is the only first-run trigger, and `once_done` is respected
  - an active run skips the poll without reporting `Conflict`
  - the plan diff against the ledger
  - the cursor and `cursor_after`
  - `retry_ids` and parking after five polls
  - the daily reconcile

  Failing test first:
  - an empty diff creates no run
  - a new document creates a run
  - the `mirror` and `new_only` item sets
  - a note added without a `modified` bump is found by the reconcile
  - a persistently failing document is parked after five polls while the cursor advances
- [ ] 4.7 Check the library identity: record the sample on first contact, compare it on every run, and return 409 on POST and on a URL change. Failing test first: pointing the source at a second fake Paperless fails `library_mismatch` with zero document changes.
- [ ] 4.8 Make trashing safe: only from a `CompleteIdSet`, never from an empty listing, and stop at the threshold, which `confirm_trash` lifts. Failing test first:
  - a failed page creates no trash items
  - an empty listing creates none and reports `empty_listing` health
  - 400 of 1,000 missing fails those items with `trash_threshold_exceeded`, and a retry with `confirm_trash` applies them

## 5. Plan phase

- [ ] 5.1 Validate field kinds before any write. Failing test first: a kind conflict fails the run and leaves the taxonomy unchanged.
- [ ] 5.2 Map taxonomy through `paperless_taxonomy`, with deterministic renames and the mirror rename collision guard. Failing test first: two plans produce the same names, and a Paperless rename renames the Folio tag.
- [ ] 5.3 Build payloads through staging, with deterministic IDs, fingerprint v1, `skipped/unchanged`, `skipped/trashed`, `needs_reapply`, and skipping documents whose `folio_state` isn't `live`. Failing test first:
  - identical payloads on a re-plan
  - a tag rename doesn't change fingerprints
  - a document with a dropped value is re-planned
  - a purged document is never re-created
- [ ] 5.4 Pre-validate values and drop invalid ones with warnings. Failing test first: an `ftp://` URL, a 20 KB note and a duplicate ASN each produce a warning.

## 6. Apply phase and driver

- [ ] 6.1 Apply `create`. Failing test first: every field of the fully tagged fixture, status `filed`, and no OCR job.
- [ ] 6.2 Apply `update` (including file replacement), `trash` and `restore`. Failing test first:
  - a bad replacement checksum keeps the old file
  - a good one switches the file and re-thumbnails
  - trash and restore round-trip
- [ ] 6.3 Fail items on checksum mismatch, missing files and oversized files. Failing test first: each reason, with the run continuing.
- [ ] 6.4 Clean up orphans on permanent create failures. Failing test first: no object is left behind.
- [ ] 6.5 Make applying crash-safe. Failing test first: a crash between `put` and the transaction, then a resume, creates one document and one object, and the ledger matches.
- [ ] 6.6 Stop crash loops. Failing test first: an item at three attempts becomes `attempts_exhausted` without being opened.
- [ ] 6.7 Resolve links through the ledger. Failing test first: links to documents from earlier runs resolve, and extra links become warnings.
- [ ] 6.8 Add `JobClass::Import` with its own worker loop, plus `JobType::ImportRun` and its handler (actor, guard re-check, batch limits, cancel check, idempotent finish). Failing test first: user uploads are thumbnailed while a 250-item import runs, and the import completes across three driver jobs.

## 7. Recovery, retry, cancel, dry run

- [ ] 7.1 Add the hook that marks the run `failed` when its job gives up. Failing test first: a run whose driver always fails ends `failed`.
- [ ] 7.2 Add the repair step for a `running` run with no job. Failing test first: re-enqueue up to 3 times, then `failed`.
- [ ] 7.3 Retry, including `confirm_trash` and the 409 while another run is active. Failing test first: only failed items are re-processed, a run that failed during planning plans again, and a retry while another run is active returns 409.
- [ ] 7.4 Cancel. Failing test first: a queued run, a run in backoff and a mid-batch run all end `cancelled`.
- [ ] 7.5 Make concurrent drivers safe. Failing test first: each item is processed once, and the counts match.
- [ ] 7.6 Dry run for both kinds. Failing test first: `would_*` statuses, a corrupted file `failed`, and nothing changed, including the cursor.

## 8. API and MCP

- [x] 8.1 Add the `[import]` config section with env overrides, `SETTINGS` entries and docs. Failing test first: the env override parses, an empty list disables export imports, and the drift test passes.
- [ ] 8.2 Add the JSON routes under `/api/vaults/{vault_id}/imports` with the guard, plus the utoipa schemas and `openapi/openapi.json` in the same commit. Failing tests first: one per spec scenario (202, 400 with identical bodies, 401, 403 `imports_disabled` and `forbidden`, 404, 409 including `library_mismatch`, 422).
- [ ] 8.3 Regenerate the Swift client from the updated spec. Failing test first: the Apple CI lane's generated-client check fails until it is regenerated.
- [ ] 8.4 Add the MCP tools with the read and write scopes, resolving the `Caller` through `CallerResolver`. Failing tests first: the four MCP scenarios.

## 9. Docs and verification

- [ ] 9.1 Write the admin guide: connecting Paperless, the sync modes and what `mirror` overwrites, the trash threshold and `confirm_trash`, the forward-only upgrade with a backup first, export imports, dry runs, retry, cancel, MCP, and deleting exports. Delegate the writing to the technical-writer agent.
- [ ] 9.2 Verify end to end with the `verify` skill:
  - Start an ephemeral Folio and a testcontainers Paperless.
  - Use `mirror` mode and import, then kill Folio mid-run and resume.
  - Edit, delete and restore in Paperless, and check that the next sync mirrors them.
  - Remove the token's view permission on part of the library and check that the threshold holds.
  - Point the source at a second Paperless and check that `library_mismatch` stops it.
  - Re-run an export of the same library and check that nothing changes.
  - Check that a user upload is processed during a large import.
  - Confirm there are no duplicates and no orphaned objects.
- [ ] 9.3 Run `make format`, `make lint` and `make test`.
