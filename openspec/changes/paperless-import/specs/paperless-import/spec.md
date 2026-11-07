## Purpose

Move an existing Paperless-ngx library into a Folio vault without losing curated metadata, keep Folio in sync while Paperless is still in use, and keep both safe to resume, retry and re-run.

## ADDED Requirements

### Requirement: Admins connect a Paperless library as a vault source

The system SHALL support a source kind `paperless` with a Paperless base URL (http or https), an API token, a sync mode (`once`, `new_only` or `mirror`, default `mirror`) and a poll interval in seconds (default 900, minimum 60). A vault SHALL have at most one live `paperless` source. Every operation on a `paperless` source (create, update, enable or disable, delete, restore, test, and the form's try-connection) SHALL require an instance admin (`is_admin` or `is_owner`) who owns the vault. With `auth.required = false`, the same rule as the settings pages applies. Tokens SHALL be redacted in responses the same way email passwords are. A changed URL SHALL require the token to be entered again. `token_file` SHALL be accepted only in `config.toml`. The HTTP client SHALL NOT follow redirects to another origin. Connection failures SHALL be reported as fixed reasons, never as raw error text.

#### Scenario: Add a connection
- **WHEN** an admin who owns the vault POSTs `{"name": "Paperless", "config": {"type": "paperless", "url": "https://paperless.example", "token": "…", "sync_mode": "mirror"}}` to `/api/sources`
- **THEN** the API returns 201 with the token redacted, and the scheduler's first poll starts the first sync run

#### Scenario: Test a connection
- **WHEN** the admin POSTs `/api/sources/{id}/test`
- **THEN** the API returns 200 with `ok: true`, the Paperless version, the number of documents the token can see, and `superuser` (whether the token sees every document), or 200 with `ok: false` and a reason (`unreachable`, `unauthorized`, `unsupported_version`, `redirected`) when it can't connect

#### Scenario: Second Paperless source in a vault
- **WHEN** the vault already has a live `paperless` source
- **THEN** POST `/api/sources` with type `paperless` returns 409

#### Scenario: Non-admin vault owner
- **WHEN** a vault owner who isn't an instance admin POSTs a `paperless` source, or calls PUT, DELETE, `/restore` or `/test` on `/api/sources/{id}` for one
- **THEN** the API returns 403 with error code `forbidden` and makes no outbound request

#### Scenario: URL changed without a token
- **WHEN** an admin PUTs `/api/sources/{id}` with a new `url` and the redacted token placeholder
- **THEN** the API returns 400 asking for the token, and the stored token is never sent to the new URL

#### Scenario: Invalid settings
- **WHEN** the URL isn't http or https, the poll interval is under 60, or the request contains `token_file`
- **THEN** POST or PUT returns 400 naming the field

#### Scenario: Source defined in config.toml
- **WHEN** `config.toml` defines `[ingest.paperless.home]` with `url`, `token_file`, `vault` and `sync_mode`
- **THEN** `GET /api/sources` returns 200 including it as config-managed, and it syncs on its interval

#### Scenario: Config source without a vault
- **WHEN** a `[ingest.paperless.*]` section has no `vault`
- **THEN** the app refuses to start and names the section

#### Scenario: Config source and managed source
- **WHEN** a vault has a config-defined Paperless source and an admin POSTs a managed one for the same vault
- **THEN** the API returns 409; the config-defined source wins

### Requirement: Admins import from an export folder

The system SHALL let an instance admin who owns the vault start an import of a Paperless-ngx `document_exporter` directory inside one of `import.allowed_roots`. With no roots configured, export imports SHALL be disabled. The request SHALL check only the path and `metadata.json`. Problems found while reading the manifest SHALL fail the run, not the request.

#### Scenario: Start an export import
- **WHEN** an admin who owns the vault POSTs `{"kind": "paperless_export", "path": "<dir>"}` to `/api/vaults/{vault_id}/imports`, `<dir>` is inside an allowed root, and `<dir>/metadata.json` names Paperless 2.16 or later
- **THEN** the API returns 202 with the run ID and status `queued`

#### Scenario: Reject a path outside the allowed roots
- **WHEN** the path, after resolving `..` and symlinks, is outside every allowed root or doesn't exist
- **THEN** POST returns 400 with the same message in both cases, and no run is created

#### Scenario: Export imports disabled
- **WHEN** `import.allowed_roots` is empty or unset
- **THEN** POST with kind `paperless_export` returns 403 with error code `imports_disabled`

#### Scenario: Reject an unsupported export
- **WHEN** `<dir>/metadata.json` is missing, isn't valid JSON, or names a Paperless version older than 2.16
- **THEN** POST returns 422 naming the problem, and no run is created

#### Scenario: Manifest unreadable after start
- **WHEN** a run starts and `manifest.json` is missing or isn't valid JSON
- **THEN** GET on the run returns 200 with status `failed` and an error that names the problem

#### Scenario: Export first, connection later
- **WHEN** a vault imported a library from an export and the admin then adds a `paperless` source for the same library
- **THEN** the first sync matches the documents by Paperless ID, recreates none of them, and only applies differences according to the sync mode

### Requirement: A vault stays bound to one Paperless library

The system SHALL record the identity of the vault's Paperless library on first contact (the Paperless ID and `added` time of up to 20 of its oldest documents) and SHALL check every later export import, sync and source change against it. When more than half of the recorded documents that are still present have a different `added` time, the library SHALL be treated as a different one.

#### Scenario: Source pointed at another Paperless
- **WHEN** an admin changes a source's URL to a different Paperless instance, or deletes the source and adds one for another instance
- **THEN** the next run fails with error `library_mismatch` before it changes any document, and POST `/api/vaults/{vault_id}/imports` for it returns 409 with `library_mismatch`

#### Scenario: Export from another library
- **WHEN** an admin starts an export import whose documents don't match the vault's recorded library
- **THEN** the run fails with `library_mismatch` and changes nothing

### Requirement: Mirror never trashes from incomplete data

In `mirror` mode the system SHALL decide that a document was deleted in Paperless only from a complete listing: every page fetched successfully and the number of IDs equal to the count Paperless reports. It SHALL NOT trash anything when the listing is empty and the ledger isn't. A run that would trash more than 50 documents or more than 10% of the ledger SHALL stop before applying any trash item and wait for an admin to confirm.

#### Scenario: Listing fails part-way
- **WHEN** fetching the document ID listing fails on any page
- **THEN** the poll records the error in the source's health and creates no `trash` items

#### Scenario: Paperless returns no documents
- **WHEN** the listing is empty but the vault's ledger has live documents
- **THEN** no document is trashed and the source's health shows `empty_listing`

#### Scenario: Token loses access to documents
- **WHEN** the token's Paperless permissions change so that 400 of 1,000 previously synced documents disappear from the listing
- **THEN** the run applies its other items and ends `completed_with_errors`, the 400 trash items are `failed` with reason `trash_threshold_exceeded`, and nothing is trashed until an admin POSTs `/api/vaults/{vault_id}/imports/{import_id}/retry` with `{"confirm_trash": true}`, which returns 202 and applies them

### Requirement: Sync keeps the vault current

A `paperless` source SHALL poll on its interval, starting with a first poll right after it is created or the app starts, and start a sync run only when it finds changes. The source SHALL keep a cursor: the highest Paperless `modified` time it has fully processed. The cursor SHALL advance when a run ends `completed` or `completed_with_errors`. Items that failed SHALL be retried by the next five polls and then parked, which lists them under the source as `parked`. A document that a Folio user trashed or purged SHALL NOT be updated, restored or re-created by sync. A mirrored note that a Folio user deleted SHALL stay deleted. The sync mode SHALL decide which changes are applied.

| Paperless change | `once` | `new_only` | `mirror` |
|---|---|---|---|
| new document | first run only | imported | imported |
| metadata, notes or content changed | — | ignored | applied, overwriting Folio edits |
| original file changed (checksum differs) | — | ignored | file replaced, thumbnail and embedding regenerated |
| document deleted or trashed | — | ignored | Folio document moved to trash |
| document restored | — | ignored | Folio document restored |
| tag, correspondent or type renamed | — | ignored | Folio entity renamed |

#### Scenario: New document in Paperless
- **WHEN** a document is added in Paperless and the source's mode is `new_only` or `mirror`
- **THEN** within one poll interval the document exists in Folio with its metadata

#### Scenario: Mirror applies an edit
- **WHEN** a user changes a Paperless document's title and tags, and a Folio user has edited the same document's title
- **THEN** after the next mirror sync the Folio document has the Paperless title and tags

#### Scenario: Mirror applies a deletion
- **WHEN** a mirrored document is deleted in Paperless
- **THEN** after the next sync the Folio document is in the trash, not purged, and restoring it in Paperless restores it in Folio

#### Scenario: Changes that don't bump Paperless's modified time
- **WHEN** a Paperless change doesn't update the document's `modified` time (for example a new note)
- **THEN** the daily full reconcile detects it by comparing the document's synced fingerprint and applies it in `mirror` mode

#### Scenario: No changes
- **WHEN** a poll finds no new, changed or deleted documents
- **THEN** no run is created and the source's health records a successful poll

#### Scenario: Paperless unreachable
- **WHEN** a poll fails because Paperless is unreachable or the token is rejected
- **THEN** no run is created, the source's health shows the error, and the next poll tries again

#### Scenario: Mode `once`
- **WHEN** a `once` source finishes its first run
- **THEN** it records that it is done, doesn't poll again even after a restart, and only syncs when an admin starts a sync with POST `/api/vaults/{vault_id}/imports` `{"kind": "paperless_source", "source_id": …}`, which returns 202

#### Scenario: A document keeps failing
- **WHEN** one document's download fails on every attempt across five polls
- **THEN** the cursor still advances, each poll retries that document, and after the fifth it is parked and no longer creates runs

#### Scenario: Folio user trashed a mirrored document
- **WHEN** a Folio user trashes a mirrored document and it then changes in Paperless
- **THEN** sync leaves it in the Folio trash and doesn't apply the change

#### Scenario: Folio user purged a mirrored document
- **WHEN** a Folio user permanently deletes a mirrored document
- **THEN** sync never re-creates it, and the ledger marks it `purged_in_folio`

#### Scenario: Source deleted
- **WHEN** an admin deletes the `paperless` source
- **THEN** polling stops and the imported documents stay in the vault

### Requirement: The sync ledger tracks what is done

For every Paperless document a vault has seen, the system SHALL record the Paperless ID, the Folio document ID, the Paperless `modified` time, the original's checksum and a fingerprint of the synced metadata. A run SHALL compute its work as the difference between the Paperless library and this ledger, and SHALL update a ledger row in the same transaction as the document change it records.

#### Scenario: Re-run the same export
- **WHEN** an export import completes and a second run imports the same export into the same vault
- **THEN** the second run creates no documents, writes no files, and GET `/api/vaults/{vault_id}/imports/{import_id}/items` returns 200 with each item `skipped` with reason `unchanged`

#### Scenario: Ledger and document agree after a crash
- **WHEN** the server stops in the middle of applying an item
- **THEN** either both the document change and its ledger row are committed or neither is, and the resumed run applies the item idempotently, so the final state is the same as after one application

### Requirement: Imports are idempotent and leave no orphans

Each imported document's Folio ID SHALL be derived from the vault and its Paperless ID. The system SHALL never hold two documents for the same Paperless ID in one vault, trashed or not. A retried item, a resumed run, or a new run SHALL NOT create duplicates or leave stored files that no document references.

#### Scenario: Resume after a crash
- **WHEN** the server stops while a run is `running` and starts again
- **THEN** the run continues with the items that weren't finished, and finished items are neither duplicated nor reapplied

#### Scenario: Crash between the file write and the database write
- **WHEN** the server stops after a document's original file is stored but before its record is written
- **THEN** no partial document is visible, and the resumed run writes to the same storage key and then writes the document with all its metadata in one step

#### Scenario: Item fails for good
- **WHEN** an item creating a document ends `failed` after its file was stored
- **THEN** the stored file is deleted

#### Scenario: Mirror file replacement fails
- **WHEN** replacing a mirrored document's original fails its checksum, or the server stops before the change commits
- **THEN** the document keeps its previous original, content and thumbnail, and the item is `failed` with reason `checksum_mismatch` or is retried

### Requirement: Runs always finish

A run SHALL NOT stay `queued` or `running` once nothing is processing it.

#### Scenario: Background job gives up
- **WHEN** the run's background job fails on all its attempts
- **THEN** GET on the run returns 200 with status `failed` and the last error, and a new run or a retry can start

#### Scenario: Background job disappears
- **WHEN** a run is `running` but no job for it is queued or active
- **THEN** within the next repair cycle (at most 5 minutes) the run is either picked up again or marked `failed`

#### Scenario: A file crashes the worker
- **WHEN** processing one item crashes the server three times in a row
- **THEN** that item is `failed` with reason `attempts_exhausted`, and the run continues with the next item

#### Scenario: Two drivers at once
- **WHEN** two background jobs for the same run are active at the same time
- **THEN** each item is processed by only one of them, and the run's counts match its items

#### Scenario: Sync while a run is active
- **WHEN** a poll finds changes while the vault already has a `queued` or `running` run
- **THEN** no second run is created, and the changes are picked up by the next poll after that run finishes

### Requirement: Curated metadata is preserved

The system SHALL import each Paperless document with its metadata and SHALL NOT replace that metadata with derived values. A single value Folio can't hold SHALL be dropped with a warning on the item, and SHALL NOT fail the document.

| Paperless | Folio |
|---|---|
| title | title |
| original file (not the archive PDF) | original file, original filename and mime type |
| content | content; OCR is not run, even when the content is empty |
| created | document date |
| added | the document's creation time |
| modified | the document's update time |
| correspondent, document type | the correspondent or type mapped to the Paperless one, created with the same name when missing |
| tags, including parent tags, excluding inbox tags | flat tags with the same name and color, created when missing |
| inbox tag on the document | the document needs review; without one it is imported as reviewed |
| custom field values | typed metadata values in fields with the same name, created when missing |
| archive serial number | value of a unique integer metadata field named `ASN` |
| notes, excluding deleted notes | document notes with their original creation time, written by the admin who started the run or created the source, or by no user for a config-defined source |

#### Scenario: Import a fully tagged document
- **WHEN** a Paperless document with a title, created date, added time, correspondent, type, three tags, two custom field values, an ASN, OCR text and a note is imported
- **THEN** GET `/api/documents/{id}` returns 200 with the same title, document date, creation time, correspondent, type, tags and content, status `filed`, the same metadata values, ASN and note, and no OCR job runs for it

#### Scenario: Inbox tag
- **WHEN** a Paperless document carries a tag marked as the inbox tag
- **THEN** the Folio document has status `needs_review` and doesn't get a tag for the inbox tag

#### Scenario: Reuse existing taxonomy
- **WHEN** the vault already has a live tag, correspondent, document type or metadata field with the same name as one in Paperless that isn't mapped yet
- **THEN** the import maps to the existing one instead of creating a second

#### Scenario: Nested tag names collide
- **WHEN** two Paperless tags share a name but have different parents
- **THEN** the one with the lower Paperless ID keeps the name, the other is imported as `<parent>/<name>`, the same names are chosen in a dry run and a real run, and the run summary lists the rename

#### Scenario: Field kind conflict
- **WHEN** the vault has a metadata field with the same name as a Paperless custom field but a kind that can't hold its values
- **THEN** the run fails with status `failed` and an error that names the field and both kinds, before it creates any taxonomy or document

#### Scenario: Value Folio can't hold
- **WHEN** a value is invalid for its Folio field (for example a URL that isn't http or https, text over the field's length limit, or an ASN another document in the vault already has), or a note is blank or over 16 KB
- **THEN** the document is imported without that value or note, and the item gets a warning that names the field and the reason

#### Scenario: Select values
- **WHEN** a custom field value is a Paperless select option ID
- **THEN** the value is stored as the option with the same label

#### Scenario: Document links
- **WHEN** a document-link value lists Paperless document IDs
- **THEN** the first ID found in the vault's sync ledger is stored as the link, and each other ID is dropped with a warning

### Requirement: Files are verified and confined

The system SHALL compare each original's checksum with the checksum Paperless reports (MD5 or SHA-256, chosen by its length), and store the SHA-256 of the bytes it saved. For export imports it SHALL read originals only from inside the export directory.

#### Scenario: Checksum mismatch
- **WHEN** an original's bytes don't match its Paperless checksum
- **THEN** the item is `failed` with reason `checksum_mismatch`, no document is created or changed, and the run continues

#### Scenario: Missing file
- **WHEN** an export manifest names an original that doesn't exist, or the Paperless API returns 404 for a document's download
- **THEN** the item is `failed` with reason `file_missing` and the run continues

#### Scenario: Unsafe file path
- **WHEN** an export manifest names an original by an absolute path, by a path containing `..`, or through a symlink
- **THEN** the item is `failed` with reason `unsafe_path`, the file isn't read, and the run continues

#### Scenario: File too large
- **WHEN** an original is larger than `import.max_file_bytes`
- **THEN** the item is `failed` with reason `file_too_large` and the run continues

### Requirement: Out-of-scope records are skipped and reported

The system SHALL import only documents, notes, tags, correspondents, document types and custom fields, and SHALL report the rest of an export instead of importing it. On the first import it SHALL NOT import documents that are trashed in Paperless.

#### Scenario: Summary lists what was dropped
- **WHEN** an export import finishes and the admin GETs `/api/vaults/{vault_id}/imports/{import_id}`
- **THEN** the API returns 200 and the summary counts the saved views, workflows, mail rules, storage paths, matching rules, users, groups, per-document permissions and share links it didn't import

#### Scenario: Trashed documents on first import
- **WHEN** an export contains documents with a `deleted_at` value and the vault has no ledger row for them
- **THEN** they are recorded as `skipped` with reason `trashed`

#### Scenario: Paperless 3 file versions
- **WHEN** a Paperless 3 document has later file versions
- **THEN** the import stores the file and content of the version with the highest index as the document's original, and records the number of skipped older versions on the item

### Requirement: Dry runs report without writing

The system SHALL accept `"dry_run": true` on POST `/api/vaults/{vault_id}/imports` for both kinds. A dry run SHALL compute the same items a real run would, read and verify every original it would write, and SHALL NOT change documents, files, taxonomy, metadata fields or the ledger.

#### Scenario: Dry run
- **WHEN** an admin who owns the vault POSTs a dry run
- **THEN** the API returns 202, the run finishes with each item `would_create`, `would_update`, `would_trash`, `would_restore`, `skipped` or `failed`, the summary lists the taxonomy it would create, reuse or rename, and nothing in the vault changes

### Requirement: Outcomes, retry and cancel are available over the API

The system SHALL record for each item its Paperless ID, its action (`create`, `update`, `trash` or `restore`), a status, a reason and warnings, and keep run-level counts that match the items. Runs SHALL record their kind (`paperless_export` or `paperless_source`), their trigger (`manual`, `schedule` or `source_created`) and their source.

#### Scenario: Read a run
- **WHEN** the vault owner GETs `/api/vaults/{vault_id}/imports/{import_id}`
- **THEN** the API returns 200 with:
  - the kind, trigger and source
  - the status (`queued`, `running`, `completed`, `completed_with_errors`, `failed` or `cancelled`)
  - the item count, counts per action and status, start and finish times, and the summary

#### Scenario: List runs
- **WHEN** the vault owner GETs `/api/vaults/{vault_id}/imports`
- **THEN** the API returns 200 with the vault's runs, newest first

#### Scenario: List failed items
- **WHEN** the vault owner GETs `/api/vaults/{vault_id}/imports/{import_id}/items?status=failed`
- **THEN** the API returns 200 with a paginated list of items, each with the Paperless ID, title, action, reason, warnings and attempt count

#### Scenario: Retry a finished run
- **WHEN** the admin POSTs `/api/vaults/{vault_id}/imports/{import_id}/retry` on a run that is `completed_with_errors`, `failed` or `cancelled`
- **THEN** the API returns 202, the run goes back to `queued`, its failed items are reset to `pending` with zero attempts, finished items are untouched, and a run that failed before its items were created starts from the beginning

#### Scenario: Retry a run that can't be retried
- **WHEN** the run is `queued`, `running` or `completed`, or another run in the vault is `queued` or `running`
- **THEN** POST `/retry` returns 409

#### Scenario: Cancel a run
- **WHEN** the admin POSTs `/api/vaults/{vault_id}/imports/{import_id}/cancel` on a `queued` or `running` run
- **THEN** the API returns 202, the run ends `cancelled` after at most the item in progress, even if its job is waiting for a retry, and finished items stay applied

#### Scenario: Cancel a finished run
- **WHEN** the run is not `queued` or `running`
- **THEN** POST `/cancel` returns 409

#### Scenario: Imports don't block normal processing
- **WHEN** an import of 5,000 documents is running and a user uploads a document
- **THEN** the uploaded document's thumbnail, OCR and embedding are processed while the import continues

#### Scenario: Reject callers who may not import
- **WHEN** an unauthenticated caller, a caller who isn't an instance admin or vault owner, or a caller who can't see the vault calls POST on `/api/vaults/{vault_id}/imports`, `/retry` or `/cancel`
- **THEN** the API returns 401, 403 with error code `forbidden`, or 404 respectively

#### Scenario: One active run per vault
- **WHEN** a vault already has a `queued` or `running` run and an admin POSTs a new one
- **THEN** the API returns 409 with the ID of the active run

#### Scenario: Unknown run
- **WHEN** the run ID doesn't exist in the vault
- **THEN** GET, `/items`, `/retry` and `/cancel` return 404

### Requirement: Imports are available over MCP

The MCP server SHALL offer the tools `list_imports`, `get_import`, `list_import_items`, `start_import`, `retry_import` and `cancel_import`, scoped to the session's vault, with the same behavior and permission rules as the JSON API. Read tools SHALL need the read scope; `start_import`, `retry_import` and `cancel_import` SHALL need the write scope.

#### Scenario: Start a sync from an MCP client
- **WHEN** an MCP client authenticated as an admin who owns the vault, with the write scope, calls `start_import` with `{"kind": "paperless_source"}`
- **THEN** the tool returns the queued run's ID and status, the same as POST `/api/vaults/{vault_id}/imports`

#### Scenario: Read-only token
- **WHEN** an MCP client with only the read scope calls `cancel_import`
- **THEN** the tool returns an error that names the missing write scope, and the run is unchanged

#### Scenario: Not an admin
- **WHEN** an MCP client authenticated as a vault owner who isn't an instance admin calls `start_import`
- **THEN** the tool returns a permission error and no run is created

#### Scenario: Inspect failures
- **WHEN** an MCP client calls `list_import_items` with `{"import_id": …, "status": "failed"}`
- **THEN** it receives the same items, reasons and warnings as GET `/api/vaults/{vault_id}/imports/{import_id}/items?status=failed`

### Requirement: Imports don't depend on an AI provider

Imports and syncs SHALL complete with the imported text whether or not an AI provider is configured. Thumbnails and embeddings for imported or replaced documents SHALL go through the normal processing pipeline after the documents are written.

#### Scenario: No AI provider configured
- **WHEN** the embedding provider is `local` or unconfigured and an import runs
- **THEN** the run completes, every imported document keeps its Paperless content, POST `/api/documents/search` returns 200 and finds the documents by text, and no OCR runs

#### Scenario: Embedding provider fails
- **WHEN** an embedding job for an imported document fails after its retries
- **THEN** the item stays `done`, and GET `/api/documents/{id}` returns 200 with the document's metadata intact
