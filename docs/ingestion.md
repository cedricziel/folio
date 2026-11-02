# Automatic ingestion

Folio can pull documents in by itself from two kinds of sources:

- a **consume folder** that Folio watches, and
- **mailboxes** it reads over IMAP.

Both hand every file to the same code path as a manual upload. The new document records where it came from (its **source**), then fires the usual `document.uploaded` workflows.

## Sources

| Source kind | `source.kind` | Details recorded                                                             |
| ----------- | ------------- | ---------------------------------------------------------------------------- |
| Web upload  | `upload`      | none                                                                         |
| JSON API    | `api`         | none                                                                         |
| Folder      | `folder`      | source name, relative path, original file name                               |
| Email       | `email`       | account, mailbox, message id, from, to, subject, date, attachment name and type |
| MCP         | `mcp`         | none (files added with the `upload_document` tool)                           |
| Share       | `share`       | reserved                                                                     |

The source is part of the document in the JSON API (`source`), in the webhook payload (`document.source`), and on the document page.

## Configuring sources

Sources can be defined in the server configuration, in the UI, or both.

- **Configuration** sources are read-only in the UI. They are the right place for secrets, because a mail password can come from a file (`password_file`, for example a Docker secret).
- **UI** sources are managed on **Settings, Sources** (`/settings/sources`) or through `/api/sources`. Their passwords are stored unencrypted in the database, and the page says so. Encrypting them needs the secret key that the authentication work introduces; until it exists, prefer `password_file` for mail accounts. `password_file` is rejected for UI sources, because it would let a UI user make the server read any file.

The page lists every source with its last run (time, imported, skipped, and failed counts) and the last error. **Test** checks a source without importing anything: a folder must be readable, a mail account must accept the login and have its mailboxes. The form has a **Test connection** button that checks the values before you save.

### Environment variables

Settings nest with a double underscore. Each source has a name of your choosing:

```sh
FOLIO_INGEST__FOLDERS__INBOX__PATH=/consume
FOLIO_INGEST__FOLDERS__INBOX__AFTER_IMPORT=move

FOLIO_INGEST__EMAIL__WORK__HOST=imap.example.com
FOLIO_INGEST__EMAIL__WORK__USERNAME=me@example.com
FOLIO_INGEST__EMAIL__WORK__PASSWORD_FILE=/run/secrets/imap_work   # same as the _FILE suffix elsewhere
FOLIO_INGEST__EMAIL__WORK__MAILBOXES='["INBOX", "Invoices"]'
```

### `config.toml`

```toml
[ingest.folders.inbox]
path = "/consume"
recursive = true            # default
subfolders_as_tags = true   # default
after_import = "move"       # "move" (default) or "delete"
settle_secs = 5             # default
rescan_secs = 60            # default

[ingest.email.work]
host = "imap.example.com"
port = 993                  # default
security = "tls"            # "tls" (default) or "plain"
accept_invalid_certs = false
username = "me@example.com"
password_file = "/run/secrets/imap_work"   # or: password = "..."
mailboxes = ["INBOX"]       # default
criteria = "UNSEEN"         # IMAP SEARCH criteria, default
poll_interval_secs = 300    # default
allowed_content_types = ["application/pdf", "image/*"]   # default
mail_body = "none"          # "eml" also stores the whole message
after_import = "mark_read"  # "mark_read" (default), "move", or "delete"
move_to = "Imported"        # required for "move"
```

Set `enabled = false` on any source to keep its settings but not run it. Changes made in the UI apply within a few seconds; configuration changes need a restart.

## Consume folder

Folio watches the folder for changes and also rescans it every `rescan_secs`, because network shares often lose change events.

- **Settling.** A file is imported only after its size and modification time stayed the same across two looks and for at least `settle_secs`. A file still being copied is left alone.
- **Ignored.** Hidden files and folders (`.name`), `~$` lock files, `Thumbs.db`, `desktop.ini`, and partial or temporary files (`.part`, `.partial`, `.tmp`, `.temp`, `.crdownload`, `.download`, `.filepart`, `.swp`, trailing `~`).
- **Tags from folders.** With `subfolders_as_tags`, `Insurance/2025/x.pdf` is imported with the tags `Insurance` and `2025`. Missing tags are created.
- **After import.** `move` puts the file into `processed/` (keeping its sub-folders; name clashes get a `-1`, `-2` suffix), `delete` removes it.
- **Failures.** A file that cannot be imported moves to `failed/` with a `<name>.error.txt` next to it saying why. Move it back to try again.
- **Skipped.** A file that a pre-ingest rule discards moves to `skipped/`.
- `processed/`, `failed/`, and `skipped/` directly inside the consume folder are never scanned.

## Mailboxes

Every `poll_interval_secs` Folio logs in, searches each mailbox with `criteria` (`UNSEEN` by default), and imports the attachments whose content type is in `allowed_content_types`. Wildcards such as `image/*` work; an attachment sent as `application/octet-stream` is judged by its file name. Inline images (signature logos) are ignored. Messages are fetched without marking them read, so a failed import leaves them unread and they are tried again.

- **Message as a document.** With `mail_body = "eml"`, the whole message is stored as an `.eml` file in addition to its attachments. Rendering a message to PDF would need a layout engine and fonts and would still lose fidelity; `.eml` keeps everything, headers and inline content included, and any mail client opens it.
- **After import.** `mark_read` sets `\Seen`. `move` uses `UID MOVE` when the server advertises MOVE. Otherwise, it requires UIDPLUS, copies the message to `move_to`, and removes only that UID with `UID EXPUNGE`. `delete` requires UIDPLUS and removes only the imported UID. These actions apply only when every attachment of the message was handled. Unrelated messages, including messages another client marked deleted, retain their flags and stay in the mailbox.
- **Unsupported cleanup.** Without UIDPLUS, Delete leaves the source message unchanged. When the server advertises neither MOVE nor UIDPLUS, Move leaves the source message unchanged and makes no destination copy. Folio logs the cleanup failure and retains the import ledger entry, so retrying cleanup does not import the document again. Rejected cleanup commands never trigger a mailbox-wide expunge.
- **Idempotency.** Folio retains the existing processed Message-ID keys. For messages without a Message-ID, a versioned key includes the source UUID, server host and port, username, security mode, mailbox, UIDVALIDITY, and UID. Reusing a UID after a mailbox reset or account change therefore does not suppress new mail. If SELECT omits UIDVALIDITY, Folio leaves messages without a Message-ID untouched and reports a failure. Older mailbox/UID fallback entries cannot identify the original message safely, so Folio ignores them. Previously handled messages without a Message-ID may be reconsidered once after upgrading. Content deduplication still prevents duplicate stored documents.
- **TLS.** Connections use implicit TLS (`security = "tls"`, usually port 993). `accept_invalid_certs` allows self-signed certificates, `plain` disables encryption and belongs on trusted networks only. STARTTLS is not supported yet.
- **Authentication.** Password login only. OAuth2 for Gmail and Microsoft 365 is planned as a later phase; until then use an app password.

On versions before the scoped IMAP cleanup fix, use **Mark Read** or disable the email source as a temporary mitigation. Move and Delete on those versions can expunge unrelated messages already marked deleted by another mail client.

## De-duplication

Every file and attachment is hashed (SHA-256) before it is imported. Content that was imported before, from any source, is skipped and counted as skipped, so a re-dropped file or a forwarded mail does not create a second document. Documents in Trash remain deduplicated. After you delete a document forever, the same content can be imported again.

An import reserves its document identity before creating the record or storing bytes. If an attempt fails, a retry resumes that identity. After a crash, a pending reservation becomes recoverable after ten minutes. Concurrent retries cannot replace a newer attempt or recreate an identity that was permanently deleted. Older pending claims without a reserved document identity remain busy for manual reconciliation, because Folio cannot safely identify their partial document.

## Rules

Rules are workflows. They see the source in two places.

### Reacting to imported documents: `document.uploaded`

Ingested documents fire `document.uploaded` like any other upload. A separate "ingested" event would duplicate it, and a rule would have to be written twice to cover manual and automatic uploads. The source conditions below narrow a workflow to the ingested ones. Tags that come from folders are already attached when the event fires.

### Discarding before import: `ingest.received`

A workflow with the trigger `ingest.received` runs before anything is stored, for each file or attachment a source found. Its only action is `skip_import`: if the conditions match, the item is discarded and nothing is created. `skip_import` is valid only for this trigger. Conditions about stored data (tag, correspondent, document type) never match here, because nothing is stored yet.

### Source conditions

All set conditions have to match, and they combine with the existing `mime_type` and `filename_glob`. Conditions on data a source does not have never match (an email condition never matches a folder file).

| Condition                | Matches against                                                    |
| ------------------------ | ------------------------------------------------------------------ |
| `source.kind`            | `upload`, `api`, `folder`, `email`, `mcp`, `share`                 |
| `source.folder_path_glob`| the folder-relative path, for example `Insurance/*`                |
| `source.email_from`      | the sender, as `Name <address>`                                    |
| `source.email_to`        | any recipient address                                              |
| `source.email_subject`   | the subject                                                        |
| `source.email_attachment_glob` | the attachment file name                                     |
| `source.email_mailbox_glob`    | the mailbox name                                             |

`email_from`, `email_to`, and `email_subject` take one of `{"glob": "..."}` (`*` and `?`, whole text), `{"contains": "..."}` (case-insensitive), or `{"regex": "..."}` (anywhere in the text). Globs are case-insensitive.

### Examples

Ignore newsletters before they are imported:

```json
{
  "name": "Ignore newsletters",
  "trigger": "ingest.received",
  "conditions": {
    "source": { "kind": "email", "email_from": { "regex": "(?i)newsletter|no-?reply@" } }
  },
  "actions": [{ "type": "skip_import" }]
}
```

Skip images from one mailbox folder (signatures, scans of junk):

```json
{
  "name": "Invoices mailbox: no images",
  "trigger": "ingest.received",
  "conditions": {
    "mime_type": "image/*",
    "source": { "email_mailbox_glob": "INBOX/Invoices" }
  },
  "actions": [{ "type": "skip_import" }]
}
```

Tag everything the tax folder delivers, and tell another system:

```json
{
  "name": "Tax documents",
  "trigger": "document.uploaded",
  "conditions": { "source": { "kind": "folder", "folder_path_glob": "Tax/*" } },
  "actions": [
    { "type": "add_tags", "tag_ids": ["<tag id>"] },
    { "type": "webhook", "url": "http://automation.local/hooks/tax" }
  ]
}
```

Set the correspondent for invoices from one shop:

```json
{
  "name": "Shop invoices",
  "trigger": "document.uploaded",
  "conditions": {
    "source": {
      "email_from": { "glob": "*@shop.example*" },
      "email_subject": { "contains": "invoice" },
      "email_attachment_glob": "*.pdf"
    }
  },
  "actions": [{ "type": "set_correspondent", "correspondent_id": "<correspondent id>" }]
}
```

## Docker

Mount the folder you want to watch into the container and point a source at it:

```yaml
services:
  folio:
    image: folio:latest
    environment:
      FOLIO_INGEST__FOLDERS__INBOX__PATH: /consume
    volumes:
      - folio-data:/data
      - /srv/scans:/consume      # a host directory, or a network share mounted on the host
```

The user Folio runs as needs read and write access to the volume: it moves or deletes imported files and creates `processed/`, `failed/`, and `skipped/` inside it. Mounting a share over NFS or SMB works because of the periodic rescan; lower `rescan_secs` if files should show up sooner. In an orchestrator, publish the same volume (for example a dataset shared with your scanner) at `/consume`.

Mail passwords belong in secrets:

```yaml
    environment:
      FOLIO_INGEST__EMAIL__WORK__HOST: imap.example.com
      FOLIO_INGEST__EMAIL__WORK__USERNAME: me@example.com
      FOLIO_INGEST__EMAIL__WORK__PASSWORD_FILE: /run/secrets/imap_work
    secrets: [imap_work]
```
