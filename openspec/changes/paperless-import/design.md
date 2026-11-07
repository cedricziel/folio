## Context

See proposal.md for motivation and specs/paperless-import/spec.md for the required behavior.

Current state that shapes the approach:

- **Jobs.** The `jobs` table gives leased, at-least-once delivery (300 s lease, 60 s heartbeat, `claim_id` fencing, `reset_stuck` re-queues expired leases). Jobs get 3 attempts with 20 s/40 s backoff and end as `failed` (`crates/folio-infra/src/queue/mod.rs`, `queue/recovery/`). The worker runs document jobs one at a time (`queue/worker.rs:228`) and workflow runs concurrently.
- **Uploads.** `Importer::import` (`crates/folio-core/src/services/importer.rs`) inserts the row with `upload_pending`, puts the object, then finalizes. `upload_pending` only defers the `Created` event; it doesn't hide the row. `CreateDocument` (`crates/folio-core/src/models/document.rs:262`) already carries content, correspondent, type and tag IDs, but no date, creation time or external reference. `created_at` is always `now`.
- **OCR skip.** The worker already skips OCR when `content` is non-empty and `force` is false (`queue/worker.rs:710`).
- **Typed metadata.** `MetadataFieldRepository`, `MetadataOptionRepository` and `MetadataValueRepository` exist in `crates/folio-core/src/ports/metadata.rs` with SQLite and PostgreSQL adapters. Notes use `DocumentEntryRepository` (`ports/document_entries.rs`).
- **Outcomes.** Ingestion sources record per-item outcomes in `ingest_outcomes`. That table's CHECKs limit it to `folder`/`email` sources, so the import gets its own tables, following the same pattern.

## Goals / Non-Goals

**Goals:**
- One import run survives restarts, worker crashes and repeated retries without duplicates or half-written documents.
- Each document lands in one database transaction together with its tags, metadata values and notes.
- Memory stays bounded for libraries with tens of thousands of documents.

**Non-Goals:**
- Faster processing of the thumbnails and embeddings that follow an import. They drain through the existing queue at its current pace. Processing concurrency and provider rate limiting are a separate change.
- Importing from a live Paperless instance over its REST API. The manifest reader port leaves room for this later.
- A web UI for imports. This change adds JSON routes only.
- Zipped exports. The admin unzips first.

## Decisions

### 1. Read `document_exporter` output, not the REST API

The exporter gives one consistent snapshot with Paperless IDs, notes, custom field instances and checksums. It needs no credentials or network access, and the server reads it from a mounted path. The REST API would need one or two requests per document, and its results depend on the token user's permissions.

Supported layouts:
- a single `manifest.json`
- `--split-manifest` (per-document `*-manifest.json` files next to the main one)
- `--use-folder-prefix`

The reader parses `metadata.json` for the version. That version decides the checksum algorithm: 32 hex characters means MD5 (v2), 64 means SHA-256 (v3).

**Port:** `folio-core::ports::import::PaperlessExportReader`, with `taxonomy()` returning tags, correspondents, types and custom fields, `documents()` returning a streaming iterator of document records with their notes and field instances, and `open_original(record)`.

**Adapter:** `folio-infra::import::paperless_fs`. It streams the manifest with `serde_json::Deserializer` over a buffered reader, so it doesn't load a multi-hundred-MB manifest into memory. It skips record models it doesn't import, and counts them for the summary. It never deserializes the `auth.user`, `paperless_mail.mailaccount` or social-token records, so credentials and password hashes don't reach memory beyond the parser.

Alternative considered: requiring `--split-manifest` only. It's simpler to stream, but most existing exports don't use it.

### 2. Two phases per run: plan, then import

- **Plan phase.** One pass over the manifest:
  - Resolves the taxonomy against the vault: tags (flattened, with collision renames), correspondents, types, metadata fields and select options.
  - Fails the run on kind conflicts before any document is written.
  - Inserts one `import_items` row per document, keyed `UNIQUE (run_id, external_id)`.
  - Pre-allocates a Folio document UUID for each item.
  - Taxonomy is created in this phase, except in a dry run.
  - The phase is idempotent: `INSERT … ON CONFLICT DO NOTHING`, and lookup by name before create.
- **Import phase.** Processes `pending` items in batches of 25, ordered by `external_id`.

Pre-allocating the UUID makes a crash between the file put and the database insert harmless: the retry writes the same storage key and inserts the same ID.

Alternative considered: a single streaming pass. It would make taxonomy conflicts fail mid-run and leave a partial import.

### 3. File first, then one transaction

For each item:
1. Open the original and hash it while reading (MD5 or SHA-256 as the manifest needs, plus SHA-256 always).
2. Compare with the manifest checksum, and fail the item on a mismatch.
3. `put` to `documents/{preallocated_id}/original`.
4. In one transaction:
   - insert the document with `file_path` set and `upload_pending = false`
   - insert its tags, metadata values and notes
   - set `import_items.status = 'imported'` and `document_id`
   - emit the `Created` event via the outbox with origin `import`

`documents.content_sha256` stores the SHA-256. The `(vault_id, external_source, external_id)` unique index is the final guard: a conflict means another attempt already finished this item, so the item is marked `skipped`/`already_imported` and the stored object is left alone, because it belongs to the existing document.

A new repository method, `DocumentRepository::create_imported(ImportedDocument)`, does the transaction. It doesn't reuse `Importer::import`, whose pending/finalize dance exists for HTTP uploads.

`ImportedDocument` extends `CreateDocument` with:
- `id`
- `external: ExternalRef { source, id }`
- `document_date`
- `created_at`
- `content_sha256`
- `metadata: Vec<MetadataValueInput>`
- `notes: Vec<ImportedNote { body, created_at }>`

Notes are written by the importing user, with `created_at` taken from Paperless.

Document links are written in a second pass after all items are `imported`, because their targets must exist. Links whose target isn't in the run are dropped and recorded as warnings.

Alternative considered: making `upload_pending` hide documents. It touches every list and search query for a benefit only the import needs.

### 4. Suppress workflows, keep processing

`CreateDocument.source` gets a new `SourceKind::Import`. The workflow trigger evaluator ignores `document.uploaded` for that source. After commit, the importer enqueues `GenerateThumbnail` and `GenerateEmbedding`. It doesn't enqueue `ProcessDocument`, so OCR is never scheduled. The existing content check would skip OCR anyway; this saves the job.

### 5. One driver job per run, resumable by state

There is a new `JobType::ImportRun { run_id }`. It isn't one job per document: thousands of rows would crowd out normal processing, and the single sequential worker loop would interleave them anyway.

- **Leasing.** The driver job runs under the existing lease. The heartbeat already covers long runs.
- **Resuming.** The run's progress lives in `import_runs`/`import_items`, not in the job. Resuming means "process items still `pending`".
- **Exhausted attempts.** If the job runs out of its 3 attempts on a systemic error (export directory unmounted), the run ends `failed`. `POST …/retry` enqueues a new driver job.
- **Run-level errors.** An item error never fails the job. The job fails only on run-level errors such as an unreadable manifest or a storage outage.
- **Item retries.** Each item has `attempts` and is retried up to 3 times within the run, then left as `failed`.
- **Fairness.** To avoid starving normal document jobs in the single worker loop, the driver processes one batch and re-enqueues itself, with `scheduled_at` set to now, until no pending items remain. Each batch is a short job, so document jobs queued meanwhile interleave.
- **Cancel.** Sets `import_runs.cancel_requested`. The driver checks it between items.

### 6. Taxonomy mapping rules

- **Tags.**
  - Folio tags are flat; each Paperless tag becomes a tag with its own name and color.
  - Documents in Paperless already carry their ancestor tags, so no hierarchy is lost from document membership.
  - Same-name tags under different parents are renamed `<parent>/<name>`.
  - `is_inbox_tag` is ignored and counted.
  - Names are matched case-sensitively, the same as the vault's unique index.
- **Correspondents and types.** Matched by name. Same-name entities owned by different Paperless users collapse into one.
- **Custom fields.** Mapped to metadata fields by `name_key`:
  - string → text
  - longtext → long_text
  - url → url
  - date → date
  - boolean → boolean
  - integer → integer
  - float → decimal (rendered with Rust's shortest round-trip formatting, then validated)
  - monetary → money (currency from the value, or `extra_data.default_currency`, otherwise the run's `default_currency` parameter, which defaults to `EUR`)
  - select → select (options by label, created in Paperless order)
  - documentlink → document_link
- **ASN.** A field named `ASN`, kind integer, `is_unique = true`. It's created when missing; a non-integer or non-unique existing `ASN` field is a kind conflict.
- **Dates.** `created` (date) → `document_date`. `added` (UTC datetime) → `created_at`. `modified` → `updated_at`.

### 7. Schema

Migration `20261107120000_paperless_import.sql`, identical version in `migrations/sqlite` and `migrations/postgres`:

- `documents`: add `external_source TEXT NULL`, `external_id TEXT NULL` and `content_sha256 TEXT NULL`. Add `UNIQUE INDEX documents_external_ref ON documents (vault_id, external_source, external_id) WHERE external_source IS NOT NULL`. The index covers trashed rows too, so restoring a trashed import doesn't collide with a re-import. A re-import of a trashed document is skipped as `already_imported`.
- `import_runs`: `id`, `vault_id`, `kind`, `source_path`, `dry_run`, `status`, `params JSON`, `summary JSON`, `counts JSON`, `error`, `cancel_requested`, `created_by`, `created_at`, `started_at`, `finished_at`. Partial unique index on `vault_id` where status is `queued` or `running`.
- `import_items`: `run_id`, `external_id`, `document_id` (pre-allocated), `title`, `status` (`pending`, `imported`, `would_import`, `skipped`, `failed`), `reason`, `warnings JSON`, `attempts`, `updated_at`. `PRIMARY KEY (run_id, external_id)`, index on `(run_id, status)`.

Existing documents keep NULL in the new columns.

### 8. API, events, config

- **Routes.** JSON routes in `crates/folio-api/src/routes/imports.rs`, under the existing vault router, guarded by the vault owner role check used in `routes/vaults.rs:155`.
- **OpenAPI.** utoipa schemas `ImportRun`, `ImportItem`, `CreateImportRequest` and `ImportItemPage` are added to `openapi/openapi.json`.
- **Swift client.** Regenerate it. No Apple UI is in scope, so the generated types are added unused.
- **Events.** New realtime events `ImportProgress { run_id, counts }`, throttled to one per second per run in the driver, plus `ImportFinished`. Both go on `/events` for clients with access to the vault.
- **Config.** `[import] allowed_roots = ["/imports"]` in `config.toml` (`FOLIO_IMPORT__ALLOWED_ROOTS`). It isn't in the settings table, because which server paths the app can read is a deployment decision, not something a web user should change. Path checks canonicalize both sides and require a prefix match on path components.

## Risks / Trade-offs

- **Thumbnail and embedding backlog after a large import** → The worker processes one document job at a time. A 5k-document import queues 10k jobs, and search by meaning lags behind until they drain. Text search works immediately because content is imported. Follow-up: configurable processing concurrency.
- **A run holds the worker between batches** → Batches of 25 bounded at about 30 s each, followed by re-enqueue, keep normal uploads responsive.
- **Large originals buffered in memory** → `StoragePort::put` takes bytes. The import reads one file at a time; the 50 MB HTTP limit doesn't apply. Files above `import.max_file_bytes` (default 512 MB) fail with `file_too_large`.
- **Exports contain secrets** → The reader skips user, mail-account and token records without deserializing them, and the docs tell admins to export with `--passphrase` and delete the export afterwards.
- **Monetary floats and Paperless rounding** → Values are parsed from Paperless's string form (`EUR123.45`), never from a float.
- **SQLite write contention with a running server** → The import runs inside the server process through the same pool. There is no second process.
- **Tag collapse across Paperless owners** → Two users' private "Invoice" tags become one shared tag. This is listed in the summary, and acceptable for a personal library.

## Migration Plan

1. Deploy a release that contains the migration. It is additive, so rollback is the previous image; the new columns and tables are ignored.
2. On hive, mount the export directory read-only into the container and set `import.allowed_roots`.
3. Run a dry run, review the summary, then run the import. Retry failed items, and leave Paperless running until the counts match.

## Open Questions

- Should `import.max_file_bytes` share the existing upload limit setting, or stay separate? Either answer fits the tasks as written.
