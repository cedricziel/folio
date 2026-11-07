# Importing from Paperless-ngx

Folio can import a Paperless-ngx library into a vault from the output of Paperless's `document_exporter`. It keeps titles, dates, correspondents, document types, tags, custom fields, ASNs, notes and the OCR text. You start and follow the import through the JSON API; there is no web UI for it.

Imports resume after a restart and never create a document twice, so you can re-run one safely.

## Before you start

- Paperless-ngx 2.16 or later. In 2.16 `created` became a date, and Folio rejects older exports with `422`. Upgrade Paperless first, then export.
- A Folio account that is an instance admin (`is_admin`, or the instance owner) and an owner of the target vault.
- An API token for that account with the `documents:write` scope (`documents:read` for the `GET` calls). See [auth](auth.md).
- Shell access to the Folio host, to mount the export and change the configuration.

## 1. Export from Paperless

```sh
docker compose exec webserver document_exporter ../export --passphrase <passphrase>
```

Always pass `--passphrase`. Without it the export holds your mail account credentials in plaintext.

Folio reads a single `manifest.json` as well as exports made with `--split-manifest` or `--use-folder-prefix`. It can't read zipped exports, so unzip one first. GPG-encrypted (v2) libraries need nothing extra: the exporter writes decrypted files.

Paperless 3 includes trashed documents in the export. Folio skips them.

## 2. Make the export visible to Folio

Mount the export directory read-only into the Folio container:

```yaml
services:
  folio:
    volumes:
      - ./export:/imports/paperless:ro
```

Then allow Folio to read from that directory. With no allowed roots, imports are off and `POST` returns `403` with `imports_disabled`.

```toml
[import]
allowed_roots = ["/imports"]
max_file_bytes = 536870912   # default, 512 MB
```

Or with environment variables (separate several roots with commas):

```sh
FOLIO_IMPORT__ALLOWED_ROOTS=/imports
FOLIO_IMPORT__MAX_FILE_BYTES=536870912
```

Originals larger than `max_file_bytes` fail with `file_too_large`. Both keys live only in the configuration, not on `/settings`. See [configuration](configuration.md) for how Folio loads them.

The import path must lie inside an allowed root after Folio resolves `..` and symlinks. A path outside every root and a path that doesn't exist get the same `400`.

Restart Folio after changing the configuration.

## 3. Do a dry run

A dry run reads the whole export and every original, and checks paths, sizes and checksums. It writes nothing: no documents, files, tags, correspondents, types or fields.

```sh
curl -X POST https://folio.example.com/api/vaults/<vault_id>/imports \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"kind": "paperless", "path": "/imports/paperless", "dry_run": true}'
```

Folio answers `202` with the run ID and status `queued`, and works through the run in the background. Poll the run until its status is no longer `queued` or `running`:

```sh
curl https://folio.example.com/api/vaults/<vault_id>/imports/<import_id> \
  -H "Authorization: Bearer <token>"
```

Each item ends `would_import`, `skipped` or `failed`, for the same reasons a real run would give. The summary lists the tags, correspondents, types and fields the import would create or reuse, any tags it would rename, and how many out-of-scope records it would leave behind.

List what failed:

```sh
curl "https://folio.example.com/api/vaults/<vault_id>/imports/<import_id>/items?status=failed" \
  -H "Authorization: Bearer <token>"
```

The list is paginated. Each item has its Paperless ID, title, reason, warnings and attempt count. Fix what you can (see [Reasons](#reasons)) and dry-run again.

## 4. Run the import

Send the same request without `dry_run`:

```sh
curl -X POST https://folio.example.com/api/vaults/<vault_id>/imports \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"kind": "paperless", "path": "/imports/paperless"}'
```

The import runs in the background in batches. There are no live progress events, so poll the run as in the dry run. If Folio restarts, the run picks up where it stopped.

A vault can have one active run at a time. A second `POST` while one is `queued` or `running` returns `409` with the active run's ID.

Running the same export again creates nothing: every item is `skipped` with `already_imported`. That includes documents you have since moved to the trash in Folio.

### Monetary fields without a currency

Paperless monetary values can lack a currency. Folio takes the currency from the value, then from the field's `extra_data.default_currency`, then from the run's `default_currency`, which defaults to `EUR`. Set it in the request:

```json
{"kind": "paperless", "path": "/imports/paperless", "default_currency": "CHF"}
```

## 5. Retry or cancel

Retry a run that ended `completed_with_errors`, `failed` or `cancelled`:

```sh
curl -X POST https://folio.example.com/api/vaults/<vault_id>/imports/<import_id>/retry \
  -H "Authorization: Bearer <token>"
```

The run goes back to `queued`. Failed items are reset to pending with zero attempts; imported and skipped items stay as they are. A run that failed before it had created its items, for example on a field kind conflict, starts over from the beginning.

Cancel a `queued` or `running` run:

```sh
curl -X POST https://folio.example.com/api/vaults/<vault_id>/imports/<import_id>/cancel \
  -H "Authorization: Bearer <token>"
```

The run stops after the item in progress and ends `cancelled`. Documents already imported stay in the vault, and so do tags, correspondents, types and fields created before a cancel or a failure.

## 6. Check the result

Thumbnails and embeddings are generated after the import by the normal processing pipeline, one document at a time. On a large library semantic search lags behind for a while. Text search works at once, because Folio imports Paperless's OCR text and doesn't run OCR again.

Imported documents don't trigger `document.uploaded` or `document.processed` [workflows](workflows.md). Later events, such as a tag change, do.

Keep Paperless running until the counts in the run match your library.

## 7. Clean up

> [!WARNING]
> Delete the export directory once the import is done and checked. It contains the password hashes of your Paperless users and, if you exported without `--passphrase`, your mail credentials in plaintext.

Then remove the volume mount and the `allowed_roots` entry, and restart Folio, so imports are off again.

## Reference

### API

All routes are under `/api/vaults/{vault_id}/imports`.

| Method and path | Does |
| --- | --- |
| `POST /` | Start a run. Body: `kind` (`paperless`), `path`, optional `dry_run`, optional `default_currency`. |
| `GET /` | List the vault's runs, newest first. |
| `GET /{import_id}` | Read one run. |
| `GET /{import_id}/items?status=failed` | List a run's items, paginated, filtered by status. |
| `POST /{import_id}/retry` | Retry a finished run. |
| `POST /{import_id}/cancel` | Cancel a queued or running run. |

| Status | When |
| --- | --- |
| `202` | Run created, retried or cancelled. |
| `400` | Path outside every allowed root, or missing. |
| `401` | Not signed in. |
| `403` `imports_disabled` | No `import.allowed_roots` configured. |
| `403` `forbidden` | Caller isn't an instance admin, or doesn't own the vault. |
| `404` | Unknown vault or run, or one the caller can't see. |
| `409` | The vault already has a `queued` or `running` run (the response carries its ID); retry on a `queued`, `running` or `completed` run; cancel on a run that isn't `queued` or `running`. |
| `422` | `metadata.json` is missing or invalid, or names Paperless older than 2.16. |

### Run status

| Status | Meaning |
| --- | --- |
| `queued` | Waiting to start. |
| `running` | Working through items. |
| `completed` | Every item imported or skipped. |
| `completed_with_errors` | Finished, with some items `failed`. |
| `failed` | The run itself failed, for example on a field kind conflict. |
| `cancelled` | Stopped on request. |

A run also reports the total number of items, counts per item status, start and finish times, and its summary.

### Reasons

| Reason | Item status | Meaning |
| --- | --- | --- |
| `already_imported` | `skipped` | The vault already has this document from an earlier run, including in the trash. |
| `trashed` | `skipped` | The document was in the Paperless trash. |
| `checksum_mismatch` | `failed` | The file doesn't match the checksum in the export. |
| `file_missing` | `failed` | The manifest names a file that isn't in the export. |
| `unsafe_path` | `failed` | The manifest names the file by an absolute path, through `..` or through a symlink. Folio doesn't read it. |
| `file_too_large` | `failed` | The file is larger than `import.max_file_bytes`. |
| `attempts_exhausted` | `failed` | The item was tried three times without finishing, for example because it crashed the process. |

### Warnings

A value Folio can't hold is dropped, and the document is still imported, with a warning that names the field and the reason. That covers:

- a URL that isn't `http` or `https`
- text over the field's length limit
- an ASN another document in the vault already has
- a blank note, or one over 16 KB
- document links beyond the first one that resolves

### Field kind conflicts

If the vault has a metadata field with the same name as a Paperless custom field but a kind that can't hold its values, the run fails before it creates anything. The error names the field and both kinds. Rename the vault field, then retry the run. An existing `ASN` field that isn't a unique integer field is also a conflict.

## What is imported

| Paperless | Folio |
| --- | --- |
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

Custom field kinds map like this:

| Paperless | Folio | Notes |
| --- | --- | --- |
| string | text | |
| longtext | long_text | |
| url | url | |
| date | date | |
| boolean | boolean | |
| integer | integer | |
| float | decimal | A value that fails validation is dropped with a warning. |
| monetary | money | Currency as described in [Monetary fields](#monetary-fields-without-a-currency). |
| select | select | Stored as the option with the same label. Options are created in Paperless order. |
| documentlink | document_link | The first ID that resolves to a document imported from the same Paperless library, in this run or an earlier one. |

Tags, correspondents, types and fields with a name that already exists in the vault are reused. Names match case-sensitively.

Folio tags are flat. When two Paperless tags share a name under different parents, the one with the lower Paperless ID keeps the name and the other becomes `<parent>/<name>`. The run summary lists these renames. Same-name tags, correspondents or types owned by different Paperless users become one.

For a Paperless 3 document with several file versions, Folio imports the latest version. The item records how many older versions it skipped.

Email originals (`.eml`) are imported without the PDF Paperless rendered from them.

## What isn't imported

Saved views, workflows, mail accounts and rules, users, groups, per-document permissions, storage paths, matching rules, audit history, share links, older file versions, archive PDFs and trashed documents. The run summary counts the records it left behind.

Folio doesn't look for duplicates of documents you uploaded by hand: a document that is both in Paperless and already in the vault ends up there twice.
