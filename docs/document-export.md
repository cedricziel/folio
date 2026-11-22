# Portable document export

Use `GET /api/documents/{id}/export` with the same credentials as the
original download. The request uses your default vault unless you supply
`X-Folio-Vault`. Readers can export documents in vaults they can access.
Documents in Trash, pending uploads, and documents without a verified
original are unavailable. For legacy originals, Folio must first complete
its original-identity backfill.

The response is an attachment named `folio-{id}.zip`. It contains:

| Entry | Contents |
| --- | --- |
| `original` | The exact uploaded bytes. Use the manifest's `original_filename` and MIME type to identify the file. Treat the filename as untrusted when restoring it to disk. |
| `archival.pdf` | The validated PDF/A copy for the exported revision, when its current conversion attempt is ready. |
| `text.txt` | Extracted text encoded as UTF-8, when stored. |
| `manifest.json` | The version 1 manifest with the document and vault IDs, title, original filename, dates, classifications, source metadata, and source revision ID. |

The manifest records the ZIP path, MIME type, byte length, and lowercase
SHA-256 digest for each payload. The archival metadata also records the
attempt ID, PDF/A profile, and the converter and validator names and versions.
The `archival` and `text` values are null when those entries are absent.
Queued, failed, skipped, superseded, or older-revision attempts contribute no
file. The manifest may contain private document and ingestion details.

Folio verifies each binary against its recorded identity and finishes the
bundle before sending it. Retry a `409` to export the current revision or
archival copy. A missing or corrupt object with a recorded verified identity
fails with `500`. Check server logs and recover the matching object from
backup. A disconnected download can be retried in full.

The limit is 512 MiB of combined binary payloads, 16 MiB of UTF-8 text,
128 KiB of stored metadata read for the document, and 1 MiB for the serialized
manifest. Larger documents return `413`. Folio admits four exports per
process, including in-progress downloads, and returns `503` with
`Retry-After` while the slots are occupied. Size the server's temporary
filesystem for roughly 2.1 GiB of ZIPs per process at maximum concurrency.
Temporary files are released on completion, failure, or cancellation.
With a reverse proxy, allow time for Folio to read and verify storage objects
before the first response byte.

A bundle is for retaining or moving a single document's files and selected
metadata. Folio has no bundle-import endpoint. Vault membership, document
history, taxonomy definitions, and server configuration require a
[deployment backup](deployment.md#back-up).
