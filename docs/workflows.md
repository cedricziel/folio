# Workflows

A workflow reacts to a document event and runs actions. Today the main action is a webhook that tells another system what happened. Manage workflows on the **Workflows** page (`/workflows`) or through the JSON API (`/api/workflows`, see the OpenAPI spec).

A workflow has a trigger, optional conditions (all must match), and ordered actions.

## Events

| Event                | Fires when                                                                 |
| -------------------- | -------------------------------------------------------------------------- |
| `document.uploaded`  | A document is created. Once per document and workflow.                     |
| `document.processed` | The background pipeline reached its last stage (embedding done or skipped). Not sent when a stage fails for good. Once per document and workflow. |
| `document.updated`   | Title, correspondent, document type, or tags changed.                      |
| `document.deleted`   | A document was deleted. Only webhook actions can run for it.               |
| `ingest.received`    | A file or email attachment was found by an ingestion source, before it is imported. Only `skip_import` runs. See [ingestion](ingestion.md). |

`uploaded` and `processed` never repeat for the same document, even if it is reprocessed. Changes made by a workflow's own actions do not trigger `document.updated` workflows.

## Conditions

Every condition you set has to match. Leave the rest empty.

- `mime_type`: exact (`application/pdf`) or with a wildcard (`image/*`)
- `tag_id`: the document has this tag
- `correspondent_id`, `document_type_id`
- `filename_glob`: `*` and `?`, case-insensitive, against the original file name
- `source`: where the document came from (folder path, email sender, subject, and so on). See [ingestion](ingestion.md#source-conditions)

## Actions

- `webhook`: `url`, `method` (`POST`), `headers`, `secret`
- `add_tags`: `tag_ids`
- `set_correspondent`: `correspondent_id`
- `set_document_type`: `document_type_id`
- `skip_import`: discard a file before import; only for `ingest.received`

Actions run in order. Each action is delivered at least once: if a later action fails and the run is retried, it resumes at the failed action and does not repeat finished ones.

## Payload

Every webhook body is JSON with this shape (`version` is the schema version; the schema is in the OpenAPI spec as `WebhookPayload`):

```json
{
  "event": "document.uploaded",
  "version": 1,
  "occurred_at": "2025-03-14T09:26:53Z",
  "document": {
    "id": "0b9c0c3e-5d5e-4a39-9f0e-0f5b3c1d2a47",
    "title": "Invoice March",
    "filename": "invoice-2025-03.pdf",
    "mime_type": "application/pdf",
    "size": 48213,
    "correspondent": { "id": "…", "name": "Stadtwerke Köln" },
    "document_type": { "id": "…", "name": "Invoice" },
    "tags": [{ "id": "…", "name": "finance" }],
    "asn": null,
    "source": { "kind": "upload" },
    "created_at": "2025-03-14T09:26:50Z",
    "urls": {
      "api": "https://folio.example.com/api/documents/0b9c0c3e-…",
      "web": "https://folio.example.com/documents/0b9c0c3e-…",
      "download": "https://folio.example.com/api/documents/0b9c0c3e-…/download"
    }
  }
}
```

Adding optional fields keeps `version` at 1. Removing or re-typing a field means a new version. `asn` is reserved and always `null` for now. `source` says how the document arrived (`upload`, `api`, `folder`, `email`, ...); for folders and mail it carries the details described in [ingestion](ingestion.md). For `document.deleted` the document is described as it was before deletion, and the links no longer resolve.

The links use `server.public_url` from `config.toml` (default `http://<host>:<port>`). Set it if receivers reach Folio under another address.

## Delivery

Each delivery is a `POST` with these headers:

| Header                | Value                                                          |
| --------------------- | -------------------------------------------------------------- |
| `Content-Type`        | `application/json`                                             |
| `User-Agent`          | `Folio-Webhooks/<version>`                                     |
| `X-Folio-Event`       | The event name                                                 |
| `X-Folio-Delivery`    | UUID of this run/action pair, identical across retries (use it to de-duplicate) |
| `X-Folio-Run`         | UUID of the workflow run, shared by all its actions             |
| `X-Folio-Timestamp`   | Unix time in seconds, fresh on every attempt                   |
| `X-Folio-Signature`   | `sha256=<hex>`, only when the workflow has a secret            |

Custom headers from the workflow are added too; they cannot replace the `X-Folio-*` ones.

De-duplicate by `X-Folio-Delivery`: distinct actions have distinct IDs even when they target the same receiver. Retries of an action retain its ID across process restarts. Use `X-Folio-Run` to correlate actions with a workflow run. Receivers that previously treated the delivery ID as a run ID must use the new run header for that lookup. Delivery IDs also change for pending runs created before this update. Let those runs finish before upgrading if your receiver needs deduplication across the transition.

### Verifying the signature

The signature is the hex HMAC-SHA256 of `<timestamp>.<raw body>` with your secret. Verify against the raw bytes, compare in constant time, and reject old timestamps to stop replays.

Python:

```python
import hashlib, hmac, time

def verify(secret: str, headers: dict, raw_body: bytes, tolerance: int = 300) -> bool:
    timestamp = headers["X-Folio-Timestamp"]
    if abs(time.time() - int(timestamp)) > tolerance:
        return False
    expected = "sha256=" + hmac.new(
        secret.encode(), timestamp.encode() + b"." + raw_body, hashlib.sha256
    ).hexdigest()
    return hmac.compare_digest(expected, headers["X-Folio-Signature"])
```

Node:

```js
import { createHmac, timingSafeEqual } from "node:crypto";

export function verify(secret, headers, rawBody, toleranceSeconds = 300) {
  const timestamp = headers["x-folio-timestamp"];
  if (Math.abs(Date.now() / 1000 - Number(timestamp)) > toleranceSeconds) return false;
  const expected =
    "sha256=" +
    createHmac("sha256", secret).update(`${timestamp}.`).update(rawBody).digest("hex");
  const given = Buffer.from(headers["x-folio-signature"] ?? "");
  const wanted = Buffer.from(expected);
  return given.length === wanted.length && timingSafeEqual(given, wanted);
}
```

## Retries

- Up to 5 attempts per run, each with a 10 second timeout.
- Network errors, timeouts, `5xx`, `408`, and `429` are retried with exponential backoff (the job queue waits 20 s, 40 s, 80 s, then 160 s).
- Other responses (for example `404` or `401`) fail the run at once. Redirects are not followed.
- The run history shows each run's status, attempts, last HTTP status, and last error. After the last attempt the run stays `failed`; Folio does not retry it again.

- A run executes the actions the workflow had when the run was created. Editing a workflow does not change runs that are already queued.
- Webhook runs have their own worker loop (four at a time), so an unreachable target never delays OCR or thumbnails. If the queue gives up on a run's job for any reason, the run is marked `failed`.

## Durability

Events are written to the database (the `outbox_events` table) in the same transaction as the document change, and a dispatcher then creates the runs. Restarting Folio between a change and its dispatch loses nothing: the pending events are handled after the next start, and handling an event twice never creates a second run. Actions that change a document (tags, correspondent, type) are announced to open browsers like any other change, but do not trigger `document.updated` workflows themselves. Dispatched events are kept for seven days.

**Send test** (`POST /api/workflows/{id}/test`) delivers a sample payload once, signed like a real one, and reports the result. It does not create a run.

## Network access

Folio is self-hosted, and the services it talks to (Home Assistant, n8n) usually sit on the same network. Webhook URLs may therefore point at private, loopback, and link-local addresses. Only `http` and `https` URLs are accepted; other schemes such as `file:` are rejected when you save a workflow and again at delivery. Anyone who can edit workflows can make the server send requests to hosts it can reach, so keep access to Folio itself restricted.

Webhook secrets are stored in the database as plain text because Folio needs them to sign each delivery. The API shows them as `********`; sending that value back on update keeps the stored one.
