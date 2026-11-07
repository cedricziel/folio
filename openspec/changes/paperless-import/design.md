## Context

See proposal.md for motivation, and specs/ for the required behavior.

Current state that shapes the approach:

- **Jobs.**
  - The `jobs` table gives leased, at-least-once delivery: 300 s lease, 60 s heartbeat, `claim_id` fencing (`crates/folio-infra/src/queue/mod.rs:188-190`).
  - `reset_stuck` re-queues expired leases. A job that runs out of attempts becomes `failed` with no hook (`queue/recovery/mod.rs:180-210`).
  - `dequeue` orders by `scheduled_at` (`queue/sqlite.rs:131-135`).
  - Document jobs run one at a time (`queue/worker.rs:226`). Jobs run under `with_actor(None)` (`worker.rs:481`).
  - The repair loop re-queues documents whose stages are pending, and it closes only workflow runs (`worker.rs:361-438`).
- **Sources.**
  - `SourceConfig` is a tagged enum (`Folder`, `Email`) in `crates/folio-core/src/models/ingest.rs:186`. The email password is write-only and can come from `password_file`.
  - `IngestScheduler` (`crates/folio-core/src/services/ingest.rs:437`) polls each enabled source on its interval and records `source_health`. Config-defined sources come from `[ingest.folders.*]` and `[ingest.email.*]`.
  - `/api/sources` has CRUD, `/test` and `/outcomes` (`crates/folio-api/src/routes/sources.rs:26-36`).
  - The `ingest_outcomes` and `source_health` CHECKs allow only `folder` and `email`.
- **Uploads.** `Importer::import` inserts the row, puts the object, then finalizes. `upload_pending` only defers the `Created` event. `CreateDocument` has no ID, dates or external reference. The repository sets `ocr_status = done` only when content is `Some`.
- **Review.** A document with `reviewed_at` NULL and finished processing is `needs_review` (`models/document.rs:248-256`).
- **Storage.** `DocumentStore::put` (`ports/storage.rs:111-116`) replaces existing objects.
- **Typed metadata.** The repositories exist (`ports/metadata.rs`). `MetadataValue::DocumentLink` holds one UUID. URLs must be http(s).
- **Notes.** `DocumentEntryRepository::create` takes no timestamp. `EntryBody::parse` rejects blank bodies and bodies over 16 KB.
- **Events and workflows.**
  - Lifecycle events go to `outbox_events` in the changing transaction.
  - The workflow engine skips events with `EventOrigin::Workflow` (`crates/folio-core/src/services/workflow_engine.rs:271`).
  - `finish_embed_stage` emits `Processed`.
- **Authorization.** Any signed-in uploader can create a vault and becomes its owner. Admin gating exists in `routes/settings.rs:37-42`. MCP tools check scopes with `require_scope` (`crates/folio-mcp/src/server.rs`).

## Goals / Non-Goals

**Goals:**
- One engine applies both export imports and connection syncs, through a source-agnostic reader port.
- Runs survive restarts, worker crashes, concurrent drivers and repeated retries without duplicates, half-written documents or orphaned files, and always reach a final status.
- Each document change lands in one transaction together with its tags, metadata values, notes, review state and ledger row.
- Memory stays bounded for libraries with tens of thousands of documents.

**Non-Goals:**
- Writing back to Paperless.
- Faster processing of thumbnails and embeddings after an import.
- Live run progress on `/events`. That contract requires a `document_id`, so clients poll the run instead.
- Web pages for runs (the designs are on the canvas; a follow-up change builds them).
- Zipped exports.

## Decisions

### 1. A source-agnostic `PaperlessReader` port with two adapters

The port lives in `folio-core::ports::import`. It provides:
- `scan()`: a stream of typed taxonomy, document, note and custom-field records
- `list_document_ids()`
- `open_original(&DocumentRecord)`
- `modified_since(cursor)`, used by the REST adapter for incremental polls; the export adapter returns everything

The record types are identical for both adapters. Anything adapter-specific (manifest paths, API URLs) sits behind an opaque handle on the record.

**Export adapter** (`folio-infra::import::paperless_fs`):
- It streams `manifest.json` with `serde_json::Deserializer`, and supports `--split-manifest` and `--use-folder-prefix`.
- It never deserializes user, mail-account, token or authenticator records.
- The original-file guard resolves every path under the canonical export directory and refuses absolute paths, `..` and symlinks.
- POST reads only `metadata.json` (version 2.16 or later).

**REST adapter** (`folio-infra::import::paperless_http`, `reqwest`):
- It sends `Authorization: Token …` and `Accept: application/json; version=9`. Version 9 is supported from 2.16 through 3.x. When the server rejects version 9, `/test` reports `unsupported_version`.
- Listing:
  - `/api/documents/?page_size=100&ordering=modified&modified__gt=<cursor>&fields=…` with `full_perms=false`.
  - The list response includes notes and custom fields, so each document needs one metadata request (`/api/documents/{id}/metadata/` for the checksum and size) and one download (`/download/?original=true`).
  - Taxonomy comes from `/api/tags|correspondents|document_types|custom_fields/`, paginated.
  - `list_document_ids()` uses `/api/documents/?fields=id&page_size=1000`.
- Trash detection: `/api/trash/` (2.10 or later).
- One request at a time per source. 429 and 5xx responses are retried with exponential backoff (1, 2, 4, 8 s, then the item fails with `upstream_error`).
- The token comes from `token` or `token_file`, and the HTTP client never logs it. The URL must be http(s).

Both adapters choose the checksum algorithm by its length: 32 hex characters means MD5, 64 means SHA-256.

### 2. One Paperless library per vault, and the ledger as the state of record

The external reference stored on documents is `('paperless', <paperless_id>)`. A vault holds at most one Paperless library, which is what lets an export import be continued by a connection. The `paperless_documents` ledger is the authority on what has been done:

| Column | Meaning |
|---|---|
| `vault_id`, `paperless_id` | Primary key |
| `document_id` | Folio ID, UUIDv5 |
| `paperless_modified` | Paperless `modified` at the last sync |
| `checksum` | Original's checksum at the last sync |
| `fingerprint` | SHA-256 over the canonical JSON of the mapped metadata: title, dates, correspondent, type, tags, values, ASN, notes and inbox flag |
| `trashed_in_paperless` | Whether Paperless had it in the trash at the last sync |
| `last_synced_at`, `last_run_id` | When and by which run it was last applied |

Each sync cycle computes items as the difference between the reader's records and the ledger:

| Condition | Action |
|---|---|
| No ledger row | `create` |
| Fingerprint differs | `update` (only in `mirror` mode) |
| Checksum differs | `update` with file replacement (only in `mirror` mode) |
| ID missing from Paperless, or trashed there | `trash` (only in `mirror` mode) |
| Trashed in the ledger but live in Paperless | `restore` (only in `mirror` mode) |

`new_only` keeps `create` only.

The ledger row is written in the same transaction as the document change. A crash can therefore never leave "document changed but ledger stale". The worst case is reapplying a change that is already applied, and applying a change is idempotent.

The `paperless_taxonomy` table maps `(vault_id, kind, paperless_id)` to a Folio entity ID and the last-synced name. Mapping happens on first sight: an existing live entity with the same name, otherwise a new one. In `mirror` mode, a Paperless rename renames the mapped Folio entity, unless another live Folio entity already has the new name; then the rename is skipped with a warning. Paperless tag, correspondent or type deletions only remove the entity from documents, through the document updates. The Folio entity stays.

The `paperless_sync_state` table holds per-source `cursor_modified`, `last_poll_at` and `last_full_reconcile_at`.

### 3. Polling through the existing scheduler

`SourceConfig::Paperless(PaperlessConfig { url, token, token_file, sync_mode, poll_interval_secs })` gets `kind()` = `SourceKind::Paperless`. It is registered with `IngestScheduler` like the folder and email sources, so health, `/test`, config-defined sources and enable/disable all come for free.

Its `poll()` doesn't push items through `IngestSink`. Instead it:
1. Returns early if the vault has an active run. This is the one-active-run rule: the changes wait for the next poll.
2. Fetches `modified_since(cursor - 60 s)` (the 60 s overlap absorbs clock skew), plus `list_document_ids()` and trash in `mirror` mode. Every 24 h, instead of the incremental fetch, it does a full scan and compares fingerprints, which catches note edits that don't bump `modified`.
3. Computes the difference against the ledger without writing anything.
4. If the difference is empty, records a successful poll and advances the cursor. Otherwise it creates an `import_runs` row (`kind = paperless_source`, `trigger = schedule`) and enqueues its driver. The cursor advances only when that run completes.

Creating a source in `once`, `new_only` or `mirror` mode queues the first full run immediately (`trigger = source_created`). A `once` source doesn't poll after that. `POST /api/vaults/{vault_id}/imports` with `{"kind": "paperless_source"}` starts a run on demand.

The `source_kind` CHECKs on `ingest_outcomes` and `source_health` gain `paperless`. Per-document outcomes for this kind live in `import_items`, not `ingest_outcomes`.

### 4. Plan phase persists a self-contained payload per item

The plan phase streams the reader once:
1. Collects taxonomy.
2. Validates field kinds. A conflict fails the run before any write.
3. Creates or maps taxonomy, in Paperless ID order for deterministic tag renames. A dry run skips this step.
4. Writes one `import_items` row per action (`ON CONFLICT (run_id, paperless_id) DO NOTHING`). The payload is the normalized JSON the apply phase needs: the original handle, checksum and size, title, content, dates and mime type, mapped IDs, values, ASN, notes, raw links, version count and the new fingerprint. Notes and custom-field instances are joined to their documents through `import_staging`, so memory stays bounded.
5. Marks unchanged items `skipped/unchanged`.
6. Drops the staging rows and sets `plan_complete`.

Every step is idempotent, so a crashed plan phase starts over. Taxonomy created before a failure stays in the vault and is listed in the summary.

For sync runs, the scheduler already computed the item set. The plan phase then only re-fetches full records for those IDs.

### 5. Apply phase: deterministic IDs, file first, then one transaction

The document ID is `Uuid::new_v5(FOLIO_IMPORT_NAMESPACE, "{vault_id}/paperless/{paperless_id}")`.

**create:**
1. Claim the item.
2. Open the original (export guard, or REST download).
3. Check the size, then hash while reading.
4. Compare the checksum.
5. `put` to `documents/{id}/original`.
6. `create_imported` in one transaction:
   - the document, with `ocr_status = done`, embed pending (or skipped when there's no text), and review state from the inbox flag
   - tags, metadata values, and notes with `external_id` and their Paperless `created_at`
   - the ledger row and `import_items.status = 'done'`, conditional on the claim
   - the `created` outbox event with `EventOrigin::Import`
7. After commit, enqueue `GenerateThumbnail` and `GenerateEmbedding`.

**update** (mirror only):
- `apply_mirror_update` overwrites the title, dates, correspondent, type, tag set, metadata values (full replace of the values set by imports) and content.
- It upserts notes by `external_id`, and soft-deletes imported notes that are gone in Paperless. Folio-native notes are untouched.
- It updates the ledger, all in one transaction with `EventOrigin::Import` events.
- **File replacement** goes to a staging key, `documents/{id}/original.incoming`. After verification, a copy to the main key and a delete of the staging key happen before the transaction. Thumbnail and embedding jobs are re-enqueued after commit. If verification fails, the main key is untouched.

**trash and restore** (mirror only): the existing trash and restore repository methods, with the ledger update in the same transaction and origin `Import`.

**Failure paths:**
- A unique conflict on the external reference during create means another attempt finished the item: it is marked `skipped/unchanged`.
- If a create item ends permanently failed after its `put`, the object is deleted, but only when no document with that ID exists.
- Values that fail validation (URL scheme, length, note body, an ASN that collides) are dropped with warnings before the transaction.
- Document links are resolved in a final pass through the ledger. Each value keeps the first resolvable link, and the rest become warnings. The pass is idempotent.

### 6. Workflows

`EventOrigin::Import` is skipped next to `EventOrigin::Workflow` in `WorkflowEngine::consume`. The workflow dispatcher also skips `document.processed` occurrences whose `EventSnapshot.source` is `import`. This is the workflow-triggers delta.

Documents carry the source `import`. The `paperless` source kind names the connection, not the document's origin. `SourceKind::Import` is added to the OpenAPI enum. The Apple client has no closed enum for it.

### 7. Item claims instead of a single driver

`JobType::ImportRun { run_id }` processes one batch, then enqueues its continuation with `scheduled_at` set to the batch's start time, so that `dequeue` picks it before the processing jobs the batch queued. A batch stops at the first of: 100 items, 60 s, or one file over 64 MB.

Each item is claimed with a conditional update:

```sql
UPDATE import_items
   SET status = 'in_progress', claim = $c, claimed_at = now(), attempts = attempts + 1
 WHERE run_id = $r AND paperless_id = $p
   AND (status = 'pending'
        OR (status = 'in_progress' AND claimed_at < now() - interval '10 minutes'))
```

Every later item write is conditional on the claim.

- **Crash loops.** An item claimed with `attempts >= 3` is marked `failed/attempts_exhausted` instead of being processed.
- **Counts.** They are computed with `GROUP BY`, so they always match the items.
- **Finishing.** It is a conditional `UPDATE … WHERE status = 'running'`, so it is idempotent.
- **Actor.** The driver runs under `with_actor(run.created_by)`. For scheduled syncs, `created_by` is the admin who created the source; config-defined sources have no actor.

**Recovery:**
- A hook where an `ImportRun` job becomes `failed` marks the run `failed`.
- A repair step re-enqueues a driver for a `running` run that has no active job, up to 3 times (`driver_restarts`), then marks it `failed`.

**Cancel** sets `cancel_requested`, which the driver checks before each item. If no driver job is active, the route moves the run to `cancelled` directly.

**Retry** resets `failed` items to `pending` with `attempts = 0`, re-plans when `plan_complete` is false, and enqueues a driver.

### 8. Taxonomy and value mapping

**Tags.** Flat. Same-name tags under different parents: the lowest Paperless ID keeps the name, and the rest become `<parent>/<name>`. Inbox tags aren't created; they drive the review state. Names match case-sensitively.

**Correspondents and types.** Mapped as in §2. Same-name entities owned by different Paperless users collapse into one.

**Custom fields:**

| Paperless | Folio | Notes |
|---|---|---|
| string | text | |
| longtext | long_text | |
| url | url | |
| date | date | |
| boolean | boolean | |
| integer | integer | |
| float | decimal | Shortest round-trip form |
| monetary | money | Parsed from `EUR123.45`. Currency from the value, else `extra_data.default_currency`, else the run's or source's `default_currency` (default `EUR`) |
| select | select | Options by label |
| documentlink | document_link | First link, see §5 |

**ASN.** A unique integer field named `ASN`.

**Dates.** `created` → `document_date`, `added` → `created_at`, `modified` → `updated_at`.

### 9. Schema

Migration `20261107120000_paperless_import.sql`, the same version in `migrations/sqlite` and `migrations/postgres`:

- **`documents`:** `external_source`, `external_id`, `content_sha256`, the paired-NULL CHECK, and `UNIQUE (vault_id, external_source, external_id) WHERE external_source IS NOT NULL`. The index covers trashed rows.
- **`document_entries`:** `external_id TEXT NULL`, with `UNIQUE (document_id, external_id) WHERE external_id IS NOT NULL`.
- **`import_runs`:**
  - `id`, `vault_id`, `kind`, `trigger`, `source_id`, `source_path`, `dry_run`, `status`
  - `plan_complete`, `cancel_requested`, `driver_restarts`
  - `params JSON`, `summary JSON`, `error`
  - `created_by`, `created_at`, `started_at`, `finished_at`
  - A partial unique index on `vault_id` where status is `queued` or `running`.
- **`import_items`:**
  - `run_id`, `paperless_id INTEGER`, `action`, `document_id`, `title`
  - `status` (`pending`, `in_progress`, `done`, `skipped`, `failed`, and the dry-run `would_*` statuses), `reason`, `warnings JSON`
  - `payload JSON`, `attempts`, `claim`, `claimed_at`, `updated_at`
  - Primary key `(run_id, paperless_id)`, with an index on `(run_id, status, paperless_id)`.
- **`import_staging`:** `run_id`, `paperless_document_id`, `kind`, `record JSON`.
- **`paperless_documents`**, **`paperless_taxonomy`** and **`paperless_sync_state`**, as in §2.
- **CHECK on `ingest_outcomes.source_kind` and `source_health.source_kind`:** gains `paperless`. SQLite can't alter a CHECK, so the table is rebuilt (create `_new`, copy, drop, rename), as `20261102200000_source_lifecycle.sql` does. PostgreSQL drops and re-adds the constraint.

Existing rows keep NULL in the new columns.

### 10. API, MCP and config

**JSON API.**
- Routes in `crates/folio-api/src/routes/imports.rs` under the vault router, with a guard that requires `owner()` plus `is_admin || is_owner`. `POST` takes either `{"kind": "paperless_export", "path", "dry_run", "default_currency"}` or `{"kind": "paperless_source", "source_id"?, "dry_run"}`. When `source_id` is omitted, the vault's Paperless source is used.
- `/api/sources` accepts `type: "paperless"`. Create, update and delete for that kind need the admin guard, and the response redacts the token as `has_token`.
- The HTML source form gets a Paperless variant: URL, token, sync mode and interval.
- utoipa schemas and paths go in the same commit as the routes, so `tests/openapi.rs` stays green. The Swift client is regenerated afterwards.

**MCP.**
- Six tools in `crates/folio-mcp/src/server.rs` call the same `ImportService` methods as the routes, using the session's vault.
- `require_scope(SCOPE_READ)` covers the list and get tools, and `SCOPE_WRITE` covers start, retry and cancel.
- The admin-plus-owner rule is enforced inside the service, so the API and MCP share it.

**Config.**
- `[import] allowed_roots`, `max_file_bytes` (`FOLIO_IMPORT__*`).
- `[ingest.paperless.<name>] url`, `token_file`, `vault`, `sync_mode`, `poll_interval_secs`, `default_currency`.
- All of these are added to `SETTINGS` and `docs/configuration.md`.
- Precedence for Paperless sources is the same as for folder and email sources.

## Risks / Trade-offs

- **Mirror overwrites Folio edits.** This is what the user chose: Paperless is the master. Overwrites only happen when the Paperless side changes, or at the daily reconcile when the fingerprint differs. Folio-native notes are kept. The admin guide says so prominently.
- **Paperless `modified` doesn't move for some changes** → The daily full reconcile with fingerprints closes the gap. Sync lag for those changes is up to 24 h.
- **Load on Paperless** → One request at a time with backoff. The first sync of 5k documents is about 10k requests. Incremental polls cost a few list requests, and mirror adds one ID listing per poll.
- **Thumbnail and embedding backlog after a large import** → Processing runs one job at a time. Text search works immediately; processing concurrency is a follow-up.
- **Large originals are buffered in memory** → One file at a time, the size cap, and the batch cut-off.
- **Secrets** → Export secrets are never deserialized. The token is write-only, file-capable and never logged. Only admins can create sources.
- **SSRF through the source URL** → Admin-only, http(s) only. Admins can already point email sources at arbitrary hosts.
- **Bursts of events** → Initial imports emit one `created` event per document. Accepted for a one-off migration.
- **Tags collapse across Paperless owners, and document links are truncated to one** → Both are listed as warnings or in the summary.

## Migration Plan

1. Deploy the release with the migration. It is additive, so rollback is the previous image.
2. Either add the Paperless source in the vault's Sources settings (URL and token, mode `mirror`), or, for a very large library, first mount an export read-only, set `import.allowed_roots`, dry-run and import it, then add the source.
3. Watch the first run, retry failures, and keep Paperless running for as long as the mirror is wanted. Switching the source to `new_only` or deleting it ends the mirroring.

## Open Questions

- Should `import.max_file_bytes` share the existing upload limit? Either answer fits the tasks.
