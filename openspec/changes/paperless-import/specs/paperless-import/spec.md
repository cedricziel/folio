## Purpose

Move an existing Paperless-ngx library into a Folio vault without losing curated metadata, and keep that move safe to resume, retry and re-run.

## ADDED Requirements

### Requirement: Vault owners start imports from allowed server paths

The system SHALL let a vault owner start an import of a Paperless-ngx `document_exporter` directory (Paperless 2.20 or 3.x) into that vault. The directory SHALL be inside one of the configured `import.allowed_roots`. With no roots configured, imports SHALL be disabled.

#### Scenario: Start an import
- **WHEN** a vault owner POSTs `{"kind": "paperless", "path": "<dir>"}` to `/api/vaults/{vault_id}/imports` and `<dir>` is inside an allowed root and contains a readable `manifest.json`
- **THEN** the API returns 202 with the run ID and status `queued`, and the run is processed in the background

#### Scenario: Reject a path outside the allowed roots
- **WHEN** the path, after resolving `..` and symlinks, is outside every allowed root
- **THEN** POST returns 400 and no run is created

#### Scenario: Imports disabled
- **WHEN** `import.allowed_roots` is empty or unset
- **THEN** POST returns 403 with a message that imports are disabled

#### Scenario: Reject an unreadable or unsupported export
- **WHEN** the directory has no `manifest.json`, the manifest isn't valid JSON, or `metadata.json` names a Paperless version older than 2.20 that has GPG-encrypted documents
- **THEN** POST returns 422 naming the problem and no run is created

#### Scenario: Reject callers who don't own the vault
- **WHEN** an unauthenticated caller, a caller without the owner role, or a caller without access to the vault POSTs to `/api/vaults/{vault_id}/imports`
- **THEN** the API returns 401, 403 or 404 respectively

#### Scenario: One active run per vault
- **WHEN** a vault already has a run that is `queued` or `running`
- **THEN** POST returns 409 with the ID of the active run

### Requirement: Imports are idempotent

Each imported document SHALL record its source (`paperless`) and Paperless document ID, and the system SHALL never hold two live documents with the same source and ID in one vault. A retried item, a resumed run, or a new run over the same export SHALL NOT create duplicates.

#### Scenario: Re-run the same export
- **WHEN** an import of an export completes and a second run imports the same export into the same vault
- **THEN** the second run creates no new documents and records each already-imported document as `skipped` with reason `already_imported`

#### Scenario: Resume after a crash
- **WHEN** the server stops while a run is `running` and starts again
- **THEN** the run continues from the items that weren't finished, and documents finished before the stop are neither duplicated nor re-written

#### Scenario: Crash between the file write and the database write
- **WHEN** the server stops after a document's original file is stored but before its record is written
- **THEN** no partial document is visible, and the resumed run reuses the same document ID and storage key, verifies the stored file, and writes the document with all its metadata in one step

#### Scenario: Same file uploaded earlier by hand
- **WHEN** an exported document's file is byte-identical to a document that was uploaded to the vault without an import
- **THEN** the import still creates the document and records a `duplicate_content` warning on the item that names the existing document

### Requirement: Curated metadata is preserved

The system SHALL import each Paperless document with its metadata, and SHALL NOT replace that metadata with derived values.

| Paperless | Folio |
|---|---|
| title | title |
| original file (not the archive PDF) | original file, original filename and mime type |
| content | content; OCR is not run |
| created | document date |
| added | the document's creation time |
| correspondent, document type | the correspondent or type with the same name in the vault, created when missing |
| tags (including inherited parent tags) | flat tags with the same name and color, created when missing |
| custom field values | typed metadata values in fields with the same name, created when missing |
| archive serial number | value of a unique integer metadata field named `ASN` |
| notes | document notes with their original creation time |

#### Scenario: Import a fully tagged document
- **WHEN** a Paperless document with a title, created date, added time, correspondent, type, three tags, two custom field values, an ASN, OCR text and a note is imported
- **THEN** the Folio document has the same title, document date, creation time, correspondent, type, tags, metadata values, ASN value, content and note, and no OCR job runs for it

#### Scenario: Reuse existing taxonomy
- **WHEN** the vault already has a live tag, correspondent, document type or metadata field with the same name as one in the export
- **THEN** the import reuses it instead of creating a second one

#### Scenario: Nested tag names collide
- **WHEN** two Paperless tags share a name but have different parents
- **THEN** the second one is imported as `<parent>/<name>`, and the run summary lists the rename

#### Scenario: Field kind conflict
- **WHEN** the vault has a metadata field with the same name as a Paperless custom field but a kind that can't hold its values
- **THEN** the run fails before importing any documents, with status `failed` and an error that names the field and both kinds

#### Scenario: Select and document-link values
- **WHEN** a custom field value is a select option ID or a list of linked Paperless document IDs
- **THEN** the select value is stored as the option with the same label, and links are stored as links to the imported Folio documents once all documents in the run exist; links to documents outside the export are dropped and listed as warnings

#### Scenario: Imported documents don't trigger upload workflows
- **WHEN** a vault has an enabled workflow on `document.uploaded` and an import creates documents in it
- **THEN** the workflow doesn't run for the imported documents

### Requirement: Files are verified before they become visible

The system SHALL compare each original file's checksum with the checksum in the export (MD5 for Paperless 2.x, SHA-256 for 3.x), store the SHA-256 of the bytes it saved, and make the document visible only after the stored file matches.

#### Scenario: Checksum mismatch
- **WHEN** an exported original's bytes don't match its manifest checksum
- **THEN** the item is `failed` with reason `checksum_mismatch`, no visible document is created, and the run continues with the next item

#### Scenario: Missing file
- **WHEN** a manifest entry names an original that doesn't exist in the export directory
- **THEN** the item is `failed` with reason `file_missing` and the run continues

### Requirement: Out-of-scope records are skipped and reported

The system SHALL import only documents, notes, tags, correspondents, document types and custom fields. It SHALL report the other parts of the export instead of importing them, and SHALL NOT import trashed documents.

#### Scenario: Summary lists what was dropped
- **WHEN** a run finishes
- **THEN** its summary counts the saved views, workflows, mail rules, storage paths, matching rules, users, groups, per-document permissions and share links it didn't import

#### Scenario: Trashed documents
- **WHEN** the export contains documents with a `deleted_at` value
- **THEN** they are recorded as `skipped` with reason `trashed`

#### Scenario: Paperless 3 file versions
- **WHEN** a Paperless 3 document has later file versions
- **THEN** the import stores the file and content of the version with the highest index as the document's original, and records the number of skipped older versions on the item

### Requirement: Dry runs report without writing

The system SHALL accept `"dry_run": true` on POST `/api/vaults/{vault_id}/imports` and SHALL then read and check the whole export without creating documents, taxonomy or metadata fields.

#### Scenario: Dry run
- **WHEN** a vault owner starts a dry run
- **THEN** the run finishes with each item marked `would_import`, `skipped` or `failed`, a summary of the tags, correspondents, types and fields it would create or reuse, and no change to the vault's documents or taxonomy

### Requirement: Progress, outcomes and retry are visible

The system SHALL record a status and reason for each item, keep run-level counts current, and publish progress on the event stream.

#### Scenario: Read a run
- **WHEN** a vault owner GETs `/api/vaults/{vault_id}/imports/{import_id}`
- **THEN** the API returns 200 with status (`queued`, `running`, `completed`, `completed_with_errors`, `failed` or `cancelled`), total items and counts per item status, start and finish times, and the summary

#### Scenario: List failed items
- **WHEN** a vault owner GETs `/api/vaults/{vault_id}/imports/{import_id}/items?status=failed`
- **THEN** the API returns 200 with a paginated list of items, each with the Paperless ID, title, reason and attempt count

#### Scenario: Retry failed items
- **WHEN** a vault owner POSTs `/api/vaults/{vault_id}/imports/{import_id}/retry` on a finished run that has failed items
- **THEN** the API returns 202, only the failed items are attempted again, and items already imported are untouched

#### Scenario: Retry a run that is still active
- **WHEN** the run is `queued` or `running`
- **THEN** POST `/retry` returns 409

#### Scenario: Cancel a run
- **WHEN** a vault owner POSTs `/api/vaults/{vault_id}/imports/{import_id}/cancel` on an active run
- **THEN** the API returns 202, the run stops after the item in progress, ends as `cancelled`, and the documents already imported stay in the vault

#### Scenario: Progress on the event stream
- **WHEN** a client is subscribed to `/events` while a run is active
- **THEN** it receives import progress events with the run ID and updated counts, at most once per second per run, and a final event when the run finishes

#### Scenario: Unknown run
- **WHEN** the run ID doesn't exist in the vault
- **THEN** GET, `/retry` and `/cancel` return 404

### Requirement: Imports don't depend on an AI provider

An import SHALL complete with the imported text whether or not an AI provider is configured. Embeddings for imported documents are queued through the normal processing pipeline.

#### Scenario: No AI provider configured
- **WHEN** the embedding provider is `local` or unconfigured and an import runs
- **THEN** the import completes, every imported document keeps its Paperless content and is found by text search, and no OCR runs

#### Scenario: Embedding provider fails
- **WHEN** an embedding job for an imported document fails after its retries
- **THEN** the import item stays `imported` and the document stays visible with its metadata
