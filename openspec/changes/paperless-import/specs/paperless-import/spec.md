## Purpose

Move an existing Paperless-ngx library into a Folio vault without losing curated metadata, and keep that move safe to resume, retry and re-run.

## ADDED Requirements

### Requirement: Admins start imports from allowed server paths

The system SHALL let a caller who is an instance admin (`is_admin` or `is_owner`) and an owner of the target vault start an import of a Paperless-ngx `document_exporter` directory into that vault. The directory SHALL be inside one of the configured `import.allowed_roots`. With no roots configured, imports SHALL be disabled. The request SHALL only check the path and `metadata.json`. Problems found while reading the manifest SHALL fail the run, not the request.

#### Scenario: Start an import
- **WHEN** an admin who owns the vault POSTs `{"kind": "paperless", "path": "<dir>"}` to `/api/vaults/{vault_id}/imports`, `<dir>` is inside an allowed root, and `<dir>/metadata.json` names Paperless 2.16 or later
- **THEN** the API returns 202 with the run ID and status `queued`, and the run is processed in the background

#### Scenario: Reject a path outside the allowed roots
- **WHEN** the path, after resolving `..` and symlinks, is outside every allowed root or doesn't exist
- **THEN** POST returns 400 with the same message in both cases, and no run is created

#### Scenario: Imports disabled
- **WHEN** `import.allowed_roots` is empty or unset
- **THEN** POST returns 403 with error code `imports_disabled`

#### Scenario: Reject an unsupported export
- **WHEN** `<dir>/metadata.json` is missing, isn't valid JSON, or names a Paperless version older than 2.16
- **THEN** POST returns 422 naming the problem, and no run is created

#### Scenario: Manifest unreadable after start
- **WHEN** a run starts and `manifest.json` is missing or isn't valid JSON
- **THEN** GET on the run returns 200 with status `failed` and an error that names the problem

#### Scenario: Reject callers who may not import
- **WHEN** an unauthenticated caller POSTs, or a caller who is not an instance admin or not an owner of the vault POSTs, or the vault doesn't exist or the caller can't see it
- **THEN** POST to `/api/vaults/{vault_id}/imports` returns 401, 403 with error code `forbidden`, or 404 respectively

#### Scenario: One active run per vault
- **WHEN** a vault already has a run that is `queued` or `running`
- **THEN** POST returns 409 with the ID of the active run

### Requirement: Imports are idempotent and leave no orphans

Each imported document SHALL record its source (`paperless`) and Paperless document ID. Its Folio ID SHALL be derived from the vault, source and Paperless ID. The system SHALL never hold two documents with the same source and Paperless ID in one vault, trashed or not. A retried item, a resumed run, or a new run over the same export SHALL NOT create duplicates or leave stored files that no document references.

#### Scenario: Re-run the same export
- **WHEN** an import of an export completes and a second run imports the same export into the same vault
- **THEN** the second run creates no new documents, writes no files, and GET `/api/vaults/{vault_id}/imports/{import_id}/items` returns 200 with each item `skipped` with reason `already_imported`

#### Scenario: Resume after a crash
- **WHEN** the server stops while a run is `running` and starts again
- **THEN** the run continues with the items that weren't finished, and documents finished before the stop are neither duplicated nor rewritten

#### Scenario: Crash between the file write and the database write
- **WHEN** the server stops after a document's original file is stored but before its record is written
- **THEN** no partial document is visible, and the resumed run writes to the same storage key and then writes the document with all its metadata in one step

#### Scenario: Item fails for good
- **WHEN** an item ends `failed` after its file was stored
- **THEN** the stored file is deleted

#### Scenario: Previously imported document is in the trash
- **WHEN** a document imported earlier is trashed and the same export is imported again
- **THEN** the item is `skipped` with reason `already_imported` and the document stays in the trash

### Requirement: Runs always finish

A run SHALL NOT stay `queued` or `running` once nothing is processing it.

#### Scenario: Background job gives up
- **WHEN** the run's background job fails on all its attempts, for example because the export directory was unmounted
- **THEN** GET on the run returns 200 with status `failed` and the last error, and a new run or a retry can start

#### Scenario: Background job disappears
- **WHEN** a run is `running` but no job for it is queued or active, for example after a crash between batches
- **THEN** within the next repair cycle (at most 5 minutes) the run is either picked up again or marked `failed`

#### Scenario: A file crashes the worker
- **WHEN** processing one item crashes the server three times in a row
- **THEN** that item is `failed` with reason `attempts_exhausted`, and the run continues with the next item

#### Scenario: Two drivers at once
- **WHEN** two background jobs for the same run are active at the same time
- **THEN** each item is processed by only one of them, and the run's counts match its items

### Requirement: Curated metadata is preserved

The system SHALL import each Paperless document with its metadata, and SHALL NOT replace that metadata with derived values. A single value that Folio can't hold SHALL be dropped with a warning on the item, and SHALL NOT fail the document.

| Paperless | Folio |
|---|---|
| title | title |
| original file (not the archive PDF) | original file, original filename and mime type |
| content | content; OCR is not run, even when the content is empty |
| created | document date |
| added | the document's creation time |
| modified | the document's update time |
| correspondent, document type | the correspondent or type with the same name in the vault, created when missing |
| tags, including parent tags, excluding inbox tags | flat tags with the same name and color, created when missing |
| inbox tag on the document | the document needs review; without one it is imported as reviewed |
| custom field values | typed metadata values in fields with the same name, created when missing |
| archive serial number | value of a unique integer metadata field named `ASN` |
| notes, excluding deleted notes | document notes written by the admin who started the run, with their original creation time |

#### Scenario: Import a fully tagged document
- **WHEN** a Paperless document with a title, created date, added time, correspondent, type, three tags, two custom field values, an ASN, OCR text and a note is imported
- **THEN** GET `/api/documents/{id}` returns 200 with the same title, document date, creation time, correspondent, type, tags and content, status `filed`, and the same metadata values, ASN and note, and no OCR job runs for it

#### Scenario: Inbox tag
- **WHEN** a Paperless document carries a tag marked as the inbox tag
- **THEN** the Folio document has status `needs_review` and doesn't get a tag for the inbox tag

#### Scenario: Reuse existing taxonomy
- **WHEN** the vault already has a live tag, correspondent, document type or metadata field with the same name as one in the export
- **THEN** the import reuses it instead of creating a second one

#### Scenario: Nested tag names collide
- **WHEN** two Paperless tags share a name but have different parents
- **THEN** the one with the lower Paperless ID keeps the name, the other is imported as `<parent>/<name>`, the same names are chosen in a dry run and a real run, and the run summary lists the rename

#### Scenario: Field kind conflict
- **WHEN** the vault has a metadata field with the same name as a Paperless custom field but a kind that can't hold its values
- **THEN** the run fails with status `failed` and an error that names the field and both kinds, before it creates any tag, correspondent, type, field or document

#### Scenario: Value Folio can't hold
- **WHEN** a value is invalid for its Folio field (for example a URL that isn't http or https, text over the field's length limit, or an ASN that another document in the vault already has), or a note is blank or over 16 KB
- **THEN** the document is imported without that value or note, and the item is `imported` with a warning that names the field and the reason

#### Scenario: Select values
- **WHEN** a custom field value is a Paperless select option ID
- **THEN** the value is stored as the option with the same label

#### Scenario: Document links
- **WHEN** a document-link value lists Paperless document IDs
- **THEN** the first ID that resolves to a document imported from the same Paperless library (in this run or an earlier one) is stored as the link, and each other ID is dropped with a warning

### Requirement: Files are verified and confined to the export

The system SHALL read each original only from inside the export directory, compare its checksum with the checksum in the export (MD5 or SHA-256, chosen by the checksum's length), and store the SHA-256 of the bytes it saved.

#### Scenario: Checksum mismatch
- **WHEN** an exported original's bytes don't match its manifest checksum
- **THEN** the item is `failed` with reason `checksum_mismatch`, no document is created, and the run continues with the next item

#### Scenario: Missing file
- **WHEN** a manifest entry names an original that doesn't exist in the export directory
- **THEN** the item is `failed` with reason `file_missing` and the run continues

#### Scenario: Unsafe file path
- **WHEN** a manifest entry names an original by an absolute path, by a path containing `..`, or through a symlink
- **THEN** the item is `failed` with reason `unsafe_path`, the file isn't read, and the run continues

#### Scenario: File too large
- **WHEN** an original is larger than `import.max_file_bytes`
- **THEN** the item is `failed` with reason `file_too_large` and the run continues

### Requirement: Out-of-scope records are skipped and reported

The system SHALL import only documents, notes, tags, correspondents, document types and custom fields. It SHALL report the other parts of the export instead of importing them, and SHALL NOT import trashed documents.

#### Scenario: Summary lists what was dropped
- **WHEN** a run finishes and its owner GETs `/api/vaults/{vault_id}/imports/{import_id}`
- **THEN** the API returns 200, and the summary counts the saved views, workflows, mail rules, storage paths, matching rules, users, groups, per-document permissions and share links the run didn't import

#### Scenario: Trashed documents
- **WHEN** the export contains documents with a `deleted_at` value
- **THEN** they are recorded as `skipped` with reason `trashed`

#### Scenario: Paperless 3 file versions
- **WHEN** a Paperless 3 document has later file versions
- **THEN** the import stores the file and content of the version with the highest index as the document's original, and records the number of skipped older versions on the item

### Requirement: Dry runs report without writing

The system SHALL accept `"dry_run": true` on POST `/api/vaults/{vault_id}/imports`. A dry run SHALL read the whole export and every original, verify paths, sizes and checksums, and SHALL NOT create documents, files, taxonomy or metadata fields.

#### Scenario: Dry run
- **WHEN** an admin who owns the vault POSTs a dry run
- **THEN** the API returns 202, the run finishes with each item `would_import`, `skipped` or `failed` for the same reasons a real run would give, the summary lists the tags, correspondents, types and fields it would create or reuse, and the vault's documents and taxonomy are unchanged

### Requirement: Outcomes, retry and cancel are available

The system SHALL record a status, reason and warnings for each item, and keep run-level counts that match the items.

#### Scenario: Read a run
- **WHEN** the vault owner GETs `/api/vaults/{vault_id}/imports/{import_id}`
- **THEN** the API returns 200 with:
  - the status: `queued`, `running`, `completed`, `completed_with_errors`, `failed` or `cancelled`
  - the total number of items and counts per item status
  - the start and finish times
  - the summary

#### Scenario: List runs
- **WHEN** the vault owner GETs `/api/vaults/{vault_id}/imports`
- **THEN** the API returns 200 with the vault's runs, newest first

#### Scenario: List failed items
- **WHEN** the vault owner GETs `/api/vaults/{vault_id}/imports/{import_id}/items?status=failed`
- **THEN** the API returns 200 with a paginated list of items, each with the Paperless ID, title, reason, warnings and attempt count

#### Scenario: Retry a finished run
- **WHEN** the admin POSTs `/api/vaults/{vault_id}/imports/{import_id}/retry` on a run that is `completed_with_errors`, `failed` or `cancelled`
- **THEN** the API returns 202, the run goes back to `queued`, its failed items are reset to `pending` with zero attempts, items that are already `imported` or `skipped` are untouched, and a run that failed before its items were created starts from the beginning

#### Scenario: Retry a run that can't be retried
- **WHEN** the run is `queued`, `running` or `completed`
- **THEN** POST `/retry` returns 409

#### Scenario: Cancel a run
- **WHEN** the admin POSTs `/api/vaults/{vault_id}/imports/{import_id}/cancel` on a `queued` or `running` run
- **THEN** the API returns 202, the run ends `cancelled` after at most the item in progress, even if its job is waiting for a retry, and the documents already imported stay in the vault

#### Scenario: Cancel a finished run
- **WHEN** the run is not `queued` or `running`
- **THEN** POST `/cancel` returns 409

#### Scenario: Unknown run
- **WHEN** the run ID doesn't exist in the vault
- **THEN** GET, `/items`, `/retry` and `/cancel` return 404

### Requirement: Imports don't depend on an AI provider

An import SHALL complete with the imported text whether or not an AI provider is configured. Thumbnails and embeddings for imported documents SHALL go through the normal processing pipeline after the documents are written.

#### Scenario: No AI provider configured
- **WHEN** the embedding provider is `local` or unconfigured and an import runs
- **THEN** the run completes, every imported document keeps its Paperless content, POST `/api/documents/search` returns 200 and finds the documents by text, and no OCR runs

#### Scenario: Embedding provider fails
- **WHEN** an embedding job for an imported document fails after its retries
- **THEN** the import item stays `imported`, and GET `/api/documents/{id}` returns 200 with the document's metadata intact
