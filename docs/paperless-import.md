# Importing from Paperless-ngx

> [!NOTE]
> This feature is being built under `openspec/changes/paperless-import`. The routes, source type and MCP tools described here don't exist yet.

Folio can move a Paperless-ngx library into a vault and keep that vault in sync while Paperless is still in use. It keeps titles, dates, correspondents, document types, tags, custom fields, ASNs, notes and the OCR text.

There are two ways in:

- **Connect Paperless** as a vault source, with its URL and an API token. Folio polls it and imports or syncs changes. Start here.
- **Import an export folder** made with Paperless's `document_exporter`. Use this for very large libraries or when Folio can't reach Paperless. You can add the connection afterwards: it matches the exported documents by Paperless ID and doesn't create them again.

Each import or sync that finds changes is an import run. Runs resume after a restart and never create a document twice, so re-running one is safe. You start and follow runs through the JSON API or MCP. Apart from the source form, there are no web pages for runs yet.

## Before you start

- Paperless-ngx 2.16 or later. Older versions fail the connection test with `unsupported_version`, and Folio rejects their exports with `422`. Upgrade Paperless first.
- A Folio account that is an instance admin (`is_admin`, or the instance owner) and an owner of the target vault. A vault owner who isn't an instance admin gets `403` `forbidden`.
- For API calls, a Folio API token with `documents:write` (`documents:read` is enough for `GET`). See [auth](auth.md).
- A vault holds one Paperless library. Once a vault has seen a library, a source or export from a different Paperless instance fails with `library_mismatch`.

## 1. Back up, then upgrade Folio

The release that adds Paperless import changes the database and can't be rolled back. The previous Folio version can't read documents and events created by an import. Back up the database before you upgrade. Rolling back means restoring that backup.

## 2. Connect Paperless

Add the source on the vault's Sources page (`/settings/sources`, see [ingestion](ingestion.md#configuring-sources)) with the Paperless URL, an API token, a sync mode and a poll interval. Or use the sources API:

```sh
curl -X POST https://folio.example.com/api/sources \
  -H "Authorization: Bearer <folio-token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "Paperless", "config": {"type": "paperless", "url": "https://paperless.example", "token": "<paperless-token>", "sync_mode": "mirror"}}'
```

Folio answers `201` with the token redacted. Its first poll starts right away and starts the first run.

| Setting | Default | Notes |
| --- | --- | --- |
| `url` | | `http` or `https` only. |
| `token` | | A Paperless API token. Redacted in every response. |
| `sync_mode` | `mirror` | `once`, `new_only` or `mirror`. See [Sync modes](#3-choose-a-sync-mode). |
| `poll_interval_secs` | 900 | Seconds between polls. At least 60. |
| `default_currency` | | Three-letter code for monetary values without one, such as `EUR`. |

Folio imports only the documents the Paperless token can see. Test the connection to check:

```sh
curl -X POST https://folio.example.com/api/sources/<source_id>/test \
  -H "Authorization: Bearer <folio-token>"
```

On success the result has `ok: true`, the Paperless version, the number of documents the token can see, and `superuser`, which is true when the token sees every document. On failure it has `ok: false` and one of these reasons:

| Reason | Meaning |
| --- | --- |
| `unreachable` | Folio can't reach the URL. |
| `unauthorized` | Paperless rejected the token. |
| `unsupported_version` | Paperless is older than 2.16. |
| `redirected` | Paperless redirected to another origin. Folio doesn't follow such redirects. |

Rules for the source:

- A vault has at most one Paperless source. A second one returns `409`.
- Changing the URL needs the token again. A `PUT` with a new `url` and the redacted token returns `400`, and Folio never sends the stored token to the new URL.
- An invalid URL, a poll interval under 60, or `token_file` in the request returns `400` naming the field.
- Deleting the source stops polling. The imported documents stay.
- A vault stays bound to the first Paperless library it saw, even after you delete the source. Pointing the source at another Paperless instance, or adding a source or export from one, fails with `library_mismatch`: `409` on the request, or the run fails before it changes anything. To import another library, use a new vault.

### In `config.toml`

```toml
[ingest.paperless.home]
url = "https://paperless.example"
token_file = "/run/secrets/paperless_token"
vault = "<vault ID>"
sync_mode = "mirror"
poll_interval_secs = 900
default_currency = "EUR"
```

`token_file` works only here, not in the API or on the Sources page. `vault` is required: without it Folio refuses to start and names the section. The source is listed on the Sources page as config-managed. A config source wins over one added in the UI, so adding a second one for the same vault returns `409`. Notes imported through a config source have no author.

## 3. Choose a sync mode

- **`once`** imports the library once, then stops polling, also after a restart. Start another sync by hand (see [Sync now](#sync-now)).
- **`new_only`** imports documents that are new in Paperless and ignores every other change.
- **`mirror`**, the default, treats Paperless as the master:

| Change in Paperless | `new_only` | `mirror` |
| --- | --- | --- |
| New document | imported | imported |
| Metadata, notes or content changed | ignored | applied, overwriting edits made in Folio |
| Original file changed | ignored | file replaced, thumbnail and embedding regenerated |
| Document deleted or trashed | ignored | Folio document moved to the trash |
| Document restored | ignored | Folio document restored |
| Tag, correspondent or type renamed | ignored | Folio entity renamed |

> [!WARNING]
> In `mirror` mode, a change in Paperless overwrites the title, tags, dates, fields and content you edited in Folio. Edit in Paperless while the mirror runs, or switch to `new_only`.

What `mirror` leaves alone:

- Notes written in Folio.
- A mirrored note that someone deleted in Folio. It stays deleted.
- Documents trashed in Folio. Sync doesn't update or restore them.
- Documents permanently deleted in Folio. Sync never creates them again.
- A rename to a name another Folio tag, correspondent or type already has. Folio skips it with a warning.
- Tags, correspondents and types deleted in Paperless. Folio removes them from the documents but keeps the entity.

Folio sees most changes at the next poll. Some changes, such as a new note, don't update the Paperless `modified` time. A daily full comparison catches those, so they can take up to 24 hours.

To stop mirroring, switch the source to `new_only` or delete it.

### Deletions are guarded

Folio moves documents to the trash only from a complete listing of Paperless document IDs. It never deletes them for good.

- If a page of the listing fails, the poll records the error in the source's health and trashes nothing.
- If Paperless returns no documents while the vault has synced ones, nothing is trashed and the source's health shows `empty_listing`.
- If a run would trash more than 50 documents or more than 10% of the synced ones, it applies everything else, ends `completed_with_errors`, and fails each trash item with `trash_threshold_exceeded`.

The threshold catches a token that lost access to part of the library. Check the token's permissions in Paperless first. If the deletions are real, confirm them:

```sh
curl -X POST https://folio.example.com/api/vaults/<vault_id>/imports/<import_id>/retry \
  -H "Authorization: Bearer <folio-token>" \
  -H "Content-Type: application/json" \
  -d '{"confirm_trash": true}'
```

The confirmation lifts the threshold for that run only.

## 4. Follow a run

Runs live under `/api/vaults/<vault_id>/imports`. List them, newest first:

```sh
curl https://folio.example.com/api/vaults/<vault_id>/imports \
  -H "Authorization: Bearer <folio-token>"
```

Read one run:

```sh
curl https://folio.example.com/api/vaults/<vault_id>/imports/<import_id> \
  -H "Authorization: Bearer <folio-token>"
```

A run reports its kind (`paperless_source` or `paperless_export`), its trigger, its source, its [status](#run-status), the item count, counts per action and status, start and finish times, and a summary. There are no live progress events; poll the run.

List the items that failed:

```sh
curl "https://folio.example.com/api/vaults/<vault_id>/imports/<import_id>/items?status=failed" \
  -H "Authorization: Bearer <folio-token>"
```

The list is paginated. Each item has its Paperless ID, title, action (`create`, `update`, `trash` or `restore`), [reason](#reasons), [warnings](#warnings) and attempt count.

Failed items in a sync are retried by the next five polls. After that they are parked and listed under the source as `parked`.

### Sync now

Start a sync without waiting for the next poll. This is the only way to sync a `once` source again.

```sh
curl -X POST https://folio.example.com/api/vaults/<vault_id>/imports \
  -H "Authorization: Bearer <folio-token>" \
  -H "Content-Type: application/json" \
  -d '{"kind": "paperless_source"}'
```

`source_id` is optional; without it Folio uses the vault's Paperless source. Folio answers `202` with the run ID and status `queued`.

A vault has one active run at a time. Starting a second one returns `409` with the active run's ID. A poll that finds changes during an active run leaves them for the next poll.

## 5. Retry or cancel

Retry a run that ended `completed_with_errors`, `failed` or `cancelled`:

```sh
curl -X POST https://folio.example.com/api/vaults/<vault_id>/imports/<import_id>/retry \
  -H "Authorization: Bearer <folio-token>"
```

The run goes back to `queued`. Failed items are reset to pending with zero attempts; finished items stay as they are. Retry returns `409` on a `queued`, `running` or `completed` run, or while another run in the vault is active.

Cancel a `queued` or `running` run:

```sh
curl -X POST https://folio.example.com/api/vaults/<vault_id>/imports/<import_id>/cancel \
  -H "Authorization: Bearer <folio-token>"
```

The run stops after the item in progress and ends `cancelled`. Items already finished stay applied, and so do tags, correspondents, types and fields created before a cancel or a failure. Cancel returns `409` on a run that isn't `queued` or `running`.

## Dry runs

Add `"dry_run": true` to a sync or export import. A dry run works out the same items a real run would, reads and verifies every original it would write, and changes nothing: no documents, files, taxonomy, fields or sync state.

```sh
curl -X POST https://folio.example.com/api/vaults/<vault_id>/imports \
  -H "Authorization: Bearer <folio-token>" \
  -H "Content-Type: application/json" \
  -d '{"kind": "paperless_source", "dry_run": true}'
```

Items end `would_create`, `would_update`, `would_trash`, `would_restore`, `skipped` or `failed`. The summary lists the tags, correspondents, types and fields the run would create, reuse or rename.

## Import from an export folder

For a very large library or an offline move, import an export first and add the connection afterwards.

### Export from Paperless

```sh
docker compose exec -T webserver document_exporter ../export --passphrase <passphrase>
```

Always pass `--passphrase`. Without it the export may hold your mail account credentials in plaintext. A passphrase given on the command line ends up in your shell history, so clear it afterwards.

To make the export smaller, you can add `--no-archive --no-thumbnail`: Folio imports neither archive PDFs nor thumbnails.

Folio reads a single `manifest.json` and exports made with `--split-manifest` or `--use-folder-prefix`. It can't import exports made with `--data-only` (they have no files) or `--zip`; unzip a zipped export first. Trashed documents in the export are skipped on the first import.

### Make the export visible to Folio

Mount the export directory read-only into the Folio container:

```yaml
services:
  folio:
    volumes:
      - ./export:/imports/paperless:ro
```

Allow Folio to read from it with [`import.allowed_roots`](configuration.md#settings), and restart Folio:

```toml
[import]
allowed_roots = ["/imports"]
```

Or `FOLIO_IMPORT__ALLOWED_ROOTS=/imports`, with several roots separated by commas. Without allowed roots, export imports are off and return `403` `imports_disabled`. Originals larger than [`import.max_file_bytes`](configuration.md#settings) (default 512 MB) fail with `file_too_large`. Both settings live only in the configuration, not on `/settings`.

The import path must lie inside an allowed root after Folio resolves `..` and symlinks. A path outside every root and a path that doesn't exist get the same `400`.

### Run the import

Dry-run first:

```sh
curl -X POST https://folio.example.com/api/vaults/<vault_id>/imports \
  -H "Authorization: Bearer <folio-token>" \
  -H "Content-Type: application/json" \
  -d '{"kind": "paperless_export", "path": "/imports/paperless", "dry_run": true}'
```

Folio checks the path and `metadata.json`, then answers `202` with the run ID and status `queued`. A missing or invalid `metadata.json`, or one from Paperless older than 2.16, returns `422`. A missing or broken `manifest.json` fails the run instead, with an error naming the problem.

[Follow the run](#4-follow-a-run), fix what you can, then send the same request without `dry_run`. The summary of a finished export import counts the saved views, workflows, mail rules, storage paths, matching rules, users, groups, per-document permissions and share links it left behind.

Importing the same export again changes nothing: each item is `skipped` with `unchanged`.

#### Monetary fields without a currency

Paperless monetary values can lack a currency. Folio takes it from the value, then from the field's `extra_data.default_currency`, then from the run's `default_currency`, which defaults to `EUR`:

```json
{"kind": "paperless_export", "path": "/imports/paperless", "default_currency": "CHF"}
```

For a config-defined source, set `default_currency` in its section.

### Delete the export

> [!WARNING]
> Delete the export directory once the import is done and checked. It contains the password hashes of your Paperless users. If you exported without `--passphrase`, it may also hold your mail credentials in plaintext.

Then remove the volume mount and the `allowed_roots` entry, and restart Folio, so export imports are off again.

## MCP

The [MCP server](mcp.md) has the tools `list_imports`, `get_import`, `list_import_items`, `start_import`, `retry_import` and `cancel_import`. They act on the session's vault and follow the same rules as the JSON API. The read tools need the read scope; `start_import`, `retry_import` and `cancel_import` need the write scope and an instance admin who owns the vault.

For example, `start_import` with `{"kind": "paperless_source"}` starts a sync, and `list_import_items` with `{"import_id": "<import_id>", "status": "failed"}` lists the failures.

## After the import

Thumbnails and embeddings are generated after the import by the normal processing pipeline, one document at a time, in the same queue as user uploads. On a large library semantic search lags behind and new uploads wait their turn. Text search works at once, because Folio imports Paperless's OCR text and doesn't run OCR again.

Imports and syncs don't start [workflows](workflows.md): not on `document.uploaded`, not on `document.processed` for the processing they queued, and not on changes a mirror sync makes, such as `document.tags_changed`. Changes users or workflows make to imported documents start workflows as usual, and so does a reprocess a user starts.

## Reference

### Imports API

All routes are under `/api/vaults/{vault_id}/imports`.

| Method and path | Does |
| --- | --- |
| `POST /` | Start a run. Export: `kind` `paperless_export`, `path`, optional `dry_run` and `default_currency`. Sync: `kind` `paperless_source`, optional `source_id` and `dry_run`. |
| `GET /` | List the vault's runs, newest first. |
| `GET /{import_id}` | Read one run. |
| `GET /{import_id}/items?status=failed` | List a run's items, paginated. |
| `POST /{import_id}/retry` | Retry a finished run. Optional body `{"confirm_trash": true}`. |
| `POST /{import_id}/cancel` | Cancel a queued or running run. |

| Status | When |
| --- | --- |
| `202` | Run started, retried or cancelled. |
| `400` | Export path outside every allowed root, or missing. |
| `401` | Not signed in. |
| `403` `imports_disabled` | Export import with no `import.allowed_roots`. |
| `403` `forbidden` | Caller isn't an instance admin, or doesn't own the vault. |
| `404` | Unknown vault or run, or one the caller can't see. |
| `409` | Another run is `queued` or `running` (the response carries its ID); `library_mismatch`; retry on a `queued`, `running` or `completed` run; cancel on a run that isn't `queued` or `running`. |
| `422` | `metadata.json` is missing or invalid, or names Paperless older than 2.16. |

### Run status

| Status | Meaning |
| --- | --- |
| `queued` | Waiting to start. |
| `running` | Working through items. |
| `completed` | Every item finished or skipped. |
| `completed_with_errors` | Finished, with some items `failed`. |
| `failed` | The run itself failed, for example on `library_mismatch`, a field kind conflict or an unreadable manifest. |
| `cancelled` | Stopped on request. |

A run never stays `running` with nothing working on it. Within 5 minutes Folio picks it up again or marks it `failed`.

### Reasons

| Reason | Item status | Meaning |
| --- | --- | --- |
| `unchanged` | `skipped` | Already synced, nothing to do. |
| `trashed` | `skipped` | In the Paperless trash on the first import. |
| `checksum_mismatch` | `failed` | The file doesn't match the checksum Paperless reports. For a mirrored file replacement, the document keeps its previous file. |
| `file_missing` | `failed` | The manifest names a file that isn't in the export, or Paperless returned `404` for the download. |
| `unsafe_path` | `failed` | The manifest names the file by an absolute path, through `..` or through a symlink. Folio doesn't read it. |
| `file_too_large` | `failed` | The file is larger than `import.max_file_bytes`. |
| `attempts_exhausted` | `failed` | The item crashed the server three times in a row. |
| `upstream_error` | `failed` | Paperless kept returning `429` or `5xx` after retries. |
| `trash_threshold_exceeded` | `failed` | The run would trash too many documents. See [Deletions are guarded](#deletions-are-guarded). |

### Warnings

A value Folio can't hold is dropped, and the document is still imported, with a warning that names the field and the reason:

- a URL that isn't `http` or `https`
- text over the field's length limit
- an ASN another document in the vault already has
- a blank note, or one over 16 KB
- document links beyond the first one Folio has synced

### Field kind conflicts

If the vault has a metadata field with the same name as a Paperless custom field but a kind that can't hold its values, the run fails before it creates any taxonomy or document. The error names the field and both kinds. Rename the vault field, then retry the run.

## What is imported

| Paperless | Folio |
| --- | --- |
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

Custom field kinds map like this:

| Paperless | Folio | Notes |
| --- | --- | --- |
| string | text | |
| longtext | long_text | |
| url | url | |
| date | date | |
| boolean | boolean | |
| integer | integer | |
| float | decimal | |
| monetary | money | Currency as in [Monetary fields](#monetary-fields-without-a-currency). |
| select | select | Stored as the option with the same label. |
| documentlink | document_link | The first linked document Folio has synced. |

Tags, correspondents, types and fields whose name already exists in the vault are reused. Names match case-sensitively.

Folio tags are flat. When two Paperless tags share a name under different parents, the one with the lower Paperless ID keeps the name and the other becomes `<parent>/<name>`. The run summary lists these renames. Same-name tags, correspondents or types owned by different Paperless users become one.

For a Paperless 3 document with several file versions, Folio imports the latest. The item records how many older versions it skipped.

Email originals (`.eml`) are imported without the PDF Paperless rendered from them.

## What isn't imported

Saved views, workflows, mail accounts and rules, users, groups, per-document permissions, storage paths, matching rules, audit history, share links, older file versions and archive PDFs. Documents in the Paperless trash are skipped on the first import. Sync is one-way: Folio never writes to Paperless.

Folio doesn't look for duplicates of documents you uploaded by hand. A document that is in Paperless and was already uploaded to the vault ends up there twice.
