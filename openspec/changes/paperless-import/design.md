## Context

See proposal.md for motivation and specs/ for the required behavior.

Current state that shapes the approach:

- **Jobs.** The `jobs` table gives leased, at-least-once delivery (`crates/folio-infra/src/queue/mod.rs:188-190`: 300 s lease, 60 s heartbeat, `claim_id` fencing). `reset_stuck` re-queues expired leases, and a job that runs out of attempts becomes `failed` with no hook (`queue/recovery/mod.rs:180-210`). `dequeue` orders by `scheduled_at` (`queue/sqlite.rs:131-135`). Document jobs run one at a time (`queue/worker.rs:226`). Jobs run under `with_actor(None)` (`worker.rs:481`). The repair loop re-queues documents whose stages are pending, and closes only abandoned workflow runs (`worker.rs:361-438`).
- **Uploads.** `Importer::import` (`crates/folio-core/src/services/importer.rs`) inserts the row with `upload_pending`, puts the object, then finalizes. `upload_pending` only defers the `Created` event; it doesn't hide the row. `CreateDocument` (`models/document.rs:262`) has no date, creation time, ID or external reference. The repository sets `ocr_status = done` only when content is `Some` (`database/sqlite/document_repo.rs:1994-1998`).
- **Review.** A document with `reviewed_at` NULL and finished processing has status `needs_review` (`models/document.rs:248-256`).
- **Storage.** `DocumentStore::put` (`ports/storage.rs:111-116`) replaces existing objects.
- **Typed metadata.** `MetadataFieldRepository`, `MetadataOptionRepository` and `MetadataValueRepository` exist (`ports/metadata.rs`). `MetadataValue::DocumentLink` holds one UUID (`models/metadata.rs:39`). URLs must be http(s) (`metadata.rs:136-150`).
- **Notes.** `DocumentEntryRepository::create` takes no timestamp and writes idempotency receipts and outbox rows (`ports/document_entries.rs:24-31`). `EntryBody::parse` rejects blank bodies and bodies over 16 KB (`models/document_entry.rs:30-35`).
- **Events and workflows.** Lifecycle events are written to `outbox_events` in the changing transaction (realtime-events spec). `finish_embed_stage` emits `Processed` (`worker.rs:820-829`). Workflow matching sees the document's source through `EventSnapshot.source`.
- **Authorization.** Any signed-in uploader can create a vault and becomes its owner (`routes/vaults.rs:209-220`). The owner check is `owner()` (`routes/vaults.rs:148`). Admin gating exists as middleware in `routes/settings.rs:37-42`.

## Goals / Non-Goals

**Goals:**
- A run survives restarts, worker crashes, concurrent drivers and repeated retries without duplicates, half-written documents or orphaned files.
- A run always reaches a final status.
- Each document lands in one database transaction together with its tags, metadata values, notes and review state.
- Memory stays bounded for libraries with tens of thousands of documents.

**Non-Goals:**
- Faster processing of the thumbnails and embeddings that follow an import. Processing concurrency and provider rate limiting are a separate change.
- Live import progress on `/events`. The realtime-events contract requires a `document_id` on every event. Clients read the run with `GET` instead.
- Importing from a live Paperless instance over its REST API. The reader port leaves room for this later.
- A web UI for imports.
- Zipped exports. The admin unzips first.

## Decisions

### 1. Read `document_exporter` output, not the REST API

The exporter gives one consistent snapshot with Paperless IDs, notes, custom field instances and checksums. It needs no credentials or network access, and the server reads it from a mounted path. The REST API would need one or two requests per document, and its results depend on the token user's permissions.

**Supported layouts:**
- a single `manifest.json`
- `--split-manifest`
- `--use-folder-prefix`

POST reads only `metadata.json`, to gate on version 2.16 or later. In 2.16, `created` became a date.

**Port and adapter:**
- **Port:** `folio-core::ports::import::PaperlessExportReader`, with `scan()` returning a streaming iterator of typed records and `open_original(relative_path)`.
- **Adapter:** `folio-infra::import::paperless_fs`.
  - It streams the manifest with `serde_json::Deserializer` over a buffered reader, so it never holds the whole manifest.
  - It counts record models it doesn't import, and never deserializes the `auth.user`, `paperless_mail.mailaccount`, social-token or authenticator records.

**The checksum algorithm** is chosen by the length of the manifest's checksum: 32 hex characters means MD5, 64 means SHA-256. It is not chosen by version, because v3 exports of documents consumed under v2 may carry either.

### 2. Plan phase persists a self-contained payload per item

The import phase must not re-read the manifest, which can be several hundred MB, for every batch. The plan phase streams the manifest once and builds everything the import phase needs.

**Steps, in order:**
1. **Collect taxonomy records.** These are small: tags, correspondents, types, custom fields and options.
2. **Validate field kinds against the vault.** Any conflict fails the run here, before anything is created.
3. **Create or reuse taxonomy** (skipped in a dry run). Each lookup is by name before create, so the step is idempotent. Tag collision renames are decided in ascending Paperless ID order, so they are deterministic.
4. **Write one `import_items` row per live document,** with `INSERT … ON CONFLICT (run_id, external_id) DO NOTHING`.
   - Notes and custom-field instances are joined to their documents through a temporary per-run table, `import_staging`, keyed by Paperless document ID, so memory stays bounded.
   - The payload is normalized JSON holding everything the import phase needs:
     - the original's relative path, checksum and size
     - title, content, the three dates and the mime type
     - mapped Folio tag, correspondent and type IDs, and the inbox flag
     - mapped metadata values and the ASN
     - notes, raw link IDs, and the number of skipped versions
   - Trashed documents get `skipped`/`trashed`.
5. **Mark already-imported items.** One query joins items to `documents` on `(vault_id, 'paperless', external_id)` and marks matches `skipped`/`already_imported`. They never reach file I/O, which also makes dry runs accurate.
6. **Drop the staging rows and set `plan_complete = true`.**

If the plan phase crashes, the retry starts it again from the top. Every step is idempotent, so this is safe. Taxonomy created before a failure or cancel stays in the vault, and the summary lists it.

Alternative considered: a single streaming pass. Taxonomy conflicts would then fail mid-run, and notes and custom fields precede documents in the manifest, so they would need buffering anyway.

### 3. Deterministic document IDs, file first, then one transaction

**Document ID.** It is `Uuid::new_v5(FOLIO_IMPORT_NAMESPACE, "{vault_id}/paperless/{external_id}")`. Every run, retry and dry run therefore agrees on the ID and storage key, so a re-put overwrites the same object instead of orphaning a new one.

**Per item:**
1. **Claim the item.** This is a committed write (§5).
2. **Open the original.** The path is resolved under the canonical export directory. Absolute paths, `..` components and symlinks are refused (`unsafe_path`), checked with `symlink_metadata` on every component.
3. **Check the size, then read and hash.** Hash with the manifest's algorithm, plus SHA-256 always.
4. **Compare checksums.**
5. **`put`** to `documents/{id}/original`.
6. **`DocumentRepository::create_imported(ImportedDocument)` in one transaction:**
   - insert the document with `file_path` set
   - set `ocr_status = done` regardless of content, and the embed stage to pending (or skipped without text)
   - set `reviewed_at`/`review_origin = system` unless the inbox flag is set
   - write tags and metadata values
   - write notes directly into `document_entries` with the Paperless `created_at` and `created_by` = run creator (not through `DocumentEntryRepository::create`, which can't take a timestamp)
   - set the item `imported` with `WHERE status = 'in_progress' AND claim = $claim`
   - write the `created` outbox event, as the realtime-events spec requires

   Values that fail validation (URL scheme, length, a note body, or an ASN that collides on `metadata_values_unique_value`) are pre-validated and dropped with item warnings before the transaction. This happens instead of failing the whole document.
7. **After commit,** enqueue `GenerateThumbnail` and `GenerateEmbedding`.

**Failure paths:**
- **Unique conflict on the external ref:** someone else finished this item, so mark it `skipped`/`already_imported`. The object at that key belongs to that document, because the IDs are equal.
- **Permanent failure after the put:** the object is deleted when no document with that ID exists.
- **Document links:** resolved in a final pass, after all items are done, by looking up `(vault_id, 'paperless', link_id)` in `documents`. This also covers documents imported by earlier runs. The pass writes the first resolvable link per value; the rest become warnings. It is idempotent, so a crash during it repeats it.

Alternative considered: pre-allocating random IDs per run. A second run would then orphan one object per document.

Alternative considered: making `upload_pending` hide documents. It touches every list and search query for a benefit only the import needs.

### 4. Workflows and events

`SourceKind::Import` is a new source kind. It is added to the OpenAPI `SourceKind` enum. The Apple client has no closed enum for it, so older builds are unaffected.

The workflow dispatcher skips `document.uploaded` and `document.processed` occurrences whose `EventSnapshot.source` is `import`; this is the workflow-triggers delta. No new `EventOrigin` is added.

The per-document `created` events still go through the outbox. Browsers will see a burst of `created` events during an import, which is acceptable for a one-off migration.

### 5. Item claims instead of a single driver

**Driver job.** `JobType::ImportRun { run_id }` processes one batch, then enqueues its continuation and completes.

**Batch size.** A batch stops at the first of these limits:
- 100 items
- 60 s
- one file over 64 MB

**Continuation order.** The continuation is enqueued with `scheduled_at` set to the time the batch started. That is earlier than the processing jobs the batch just queued, so `dequeue` picks the continuation first. Imports don't wait behind their own thumbnails and embeddings, and normal uploads wait at most one batch.

**Item claims.** Correctness doesn't depend on there being one driver. Each item is claimed with:

```sql
UPDATE import_items
   SET status = 'in_progress', claim = $c, claimed_at = now(), attempts = attempts + 1
 WHERE run_id = $r AND external_id = $e
   AND (status = 'pending'
        OR (status = 'in_progress' AND claimed_at < now() - interval '10 minutes'))
```

**How this covers the failure cases:**
- **Two drivers.** If a race or lease expiry leaves two drivers, they share the remaining items without overlap. Every later item update is conditional on the claim.
- **Crash loop.** An item claimed with `attempts >= 3` is marked `failed`/`attempts_exhausted` instead of being processed. A file that crashes the process three times is skipped.
- **Counts.** Run counts are computed from `import_items` with `GROUP BY status`, not incremented. Duplicate drivers can't skew them.
- **Finishing.** It is a conditional `UPDATE import_runs SET status = … WHERE status = 'running'`, so it is idempotent.
- **Actor.** Jobs normally run with no actor. The driver runs under `with_actor(run.created_by)`, so `created_by` and `updated_by` on taxonomy, documents and notes name the admin.

**Recovery.**
- **Job gives up.** When an `ImportRun` job exhausts its attempts, a new hook in `queue/recovery`, called where the job becomes `failed`, marks the run `failed` with the job's last error.
- **Repair loop.** A new step looks for `running` runs with no active `ImportRun` job. It re-enqueues a driver up to 3 times per run (`import_runs.driver_restarts`), then marks the run `failed`.
- **Cancel.**
  - `POST …/cancel` sets `cancel_requested`. The driver checks it before each item.
  - If no driver job is active, which covers `queued` runs and jobs waiting on backoff, the route itself moves the run to `cancelled` in the same transaction.
- **Retry.**
  - `POST …/retry` resets `failed` items to `pending` with `attempts = 0`, and sets the run to `queued`.
  - If the plan phase never completed (`plan_complete = false`), the run restarts it.
  - The route then enqueues a driver.

### 6. Taxonomy mapping rules

**Tags.**
- Folio tags are flat; each Paperless tag becomes a tag with its own name and color.
- Documents in Paperless already carry their ancestor tags, so no hierarchy is lost from document membership.
- Same-name tags under different parents: the lowest Paperless ID keeps the name, the others become `<parent>/<name>`.
- Inbox tags are not created; documents carrying one are left unreviewed.
- Names match case-sensitively, the same as the vault's unique index.

**Correspondents and types.** Matched by name. Same-name entities owned by different Paperless users collapse into one.

**Custom fields.** Mapped to metadata fields by `name_key`:

| Paperless | Folio | Notes |
|---|---|---|
| string | text | |
| longtext | long_text | |
| url | url | |
| date | date | |
| boolean | boolean | |
| integer | integer | |
| float | decimal | Shortest round-trip form, then validated; a value that fails validation is dropped with a warning |
| monetary | money | Parsed from the Paperless string `EUR123.45`, never a float. Currency from the value, else `extra_data.default_currency`, else the run's `default_currency` parameter (default `EUR`) |
| select | select | Options by label, created in Paperless order |
| documentlink | document_link | First resolvable link, see §3 |

**ASN.** A field named `ASN`, kind integer, `is_unique = true`. It's created when missing. An existing `ASN` field that isn't a unique integer field is a kind conflict.

**Dates.**
- `created` (a date) → `document_date`
- `added` (UTC) → `created_at`
- `modified` → `updated_at`

### 7. Schema

Migration `20261107120000_paperless_import.sql`, identical version in `migrations/sqlite` and `migrations/postgres`:

- **`documents`:** add `external_source TEXT NULL`, `external_id TEXT NULL` and `content_sha256 TEXT NULL`.
  - `CHECK ((external_source IS NULL) = (external_id IS NULL))`.
  - `UNIQUE INDEX documents_external_ref ON documents (vault_id, external_source, external_id) WHERE external_source IS NOT NULL`. The index includes trashed rows, so a trashed import is never duplicated.
- **`import_runs`:**
  - `id`, `vault_id`, `kind`, `source_path`, `dry_run`, `status`
  - `plan_complete`, `cancel_requested`, `driver_restarts`
  - `params JSON`, `summary JSON`, `error`
  - `created_by`, `created_at`, `started_at`, `finished_at`
  - A partial unique index on `vault_id` where status is `queued` or `running`.
- **`import_items`:**
  - `run_id`, `external_id`, `seq INTEGER` (the numeric Paperless ID, for ordering), `document_id`, `title`
  - `status` (`pending`, `in_progress`, `imported`, `would_import`, `skipped`, `failed`), `reason`, `warnings JSON`
  - `payload JSON`, `attempts`, `claim`, `claimed_at`, `updated_at`
  - `PRIMARY KEY (run_id, external_id)`, with an index on `(run_id, status, seq)`.
- **`import_staging`:** `run_id`, `paperless_document_id`, `kind`, `record JSON`. It is emptied when the plan phase completes.

Existing documents keep NULL in the new columns.

### 8. API and config

**Routes.**
- JSON routes in `crates/folio-api/src/routes/imports.rs`, under the vault router.
- A guard that requires both the vault `owner()` check and `is_admin || is_owner` on the user.
- Disabled imports return 403 `imports_disabled`. Other refusals return 403 `forbidden`.
- A path that is outside every root or doesn't exist returns the same 400. The canonicalize error is never echoed, so the response doesn't reveal which paths exist.

**OpenAPI.** utoipa schemas `ImportRun`, `ImportItem`, `CreateImportRequest` and `ImportItemPage` go into `openapi/openapi.json` in the same commit as the routes, which keeps `tests/openapi.rs` green. Then regenerate the Swift client.

**Config.**
- The section is `[import]`, with `allowed_roots = ["/imports"]` and `max_file_bytes = 536870912`, overridden by `FOLIO_IMPORT__ALLOWED_ROOTS` and `FOLIO_IMPORT__MAX_FILE_BYTES`.
- Both are added to `SETTINGS` (`config.rs:471`) and `docs/configuration.md`, which the drift test checks.
- They're config-only. Which server paths the app reads is a deployment decision, not a web setting.
- The allowed-root check runs at POST and again at every driver start, so a narrowed config takes effect on resume.

## Risks / Trade-offs

- **Thumbnail and embedding backlog after a large import** → A 5k-document import queues about 10k processing jobs that drain one at a time. Search by meaning lags behind until they finish, but text search works immediately because the content is imported. The follow-up is configurable processing concurrency.
- **Large originals are buffered in memory** → `DocumentStore::put` takes bytes. Imports read one file at a time, the size cap bounds each file, and a batch ends after any file over 64 MB.
- **The export contains secrets** → The reader skips user, mail-account and token records without deserializing them. Only instance admins can import. The admin guide says to export with `--passphrase` and delete the export afterwards.
- **A burst of `created` events** → Live browsers re-render the list once per event during an import. This is accepted for a one-off migration.
- **Tags collapse across Paperless owners** → Two users' private "Invoice" tags become one shared tag. The summary lists it, and this is acceptable for a personal library.
- **Document links are truncated to the first link** → Folio holds one link per value. Every dropped link becomes a warning.
- **Imported documents never fire `document.processed` workflows**, even on a later reprocess → This is documented in the workflow-triggers delta.
- **Email originals** → `.eml` files are imported as originals without Paperless's rendered PDF. Folio thumbnails what it can.

## Migration Plan

1. Deploy a release that contains the migration. It is additive, so rollback is the previous image; the new columns and tables are ignored.
2. On hive, mount the export directory read-only into the container and set `import.allowed_roots`.
3. Run a dry run and review the summary, then run the import. Retry failed items, and leave Paperless running until the counts match.

## Open Questions

- Should `import.max_file_bytes` share the existing upload limit setting, or stay separate? Either answer fits the tasks as written.
