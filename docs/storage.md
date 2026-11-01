# Storage

Folio keeps uploaded files, thumbnails and page previews in an object store.
Two backends exist: the local filesystem (default) and any S3-compatible
service. Select one with `FOLIO_STORAGE__BACKEND=local|s3`.

## Object keys

Everything is addressed by an opaque key. The database stores keys, never
filesystem paths.

| Key | Content |
| --- | --- |
| `documents/{id}/original` | The uploaded file |
| `documents/{id}/thumb.jpg` | First-page thumbnail (also page 1 in the viewer) |
| `documents/{id}/pages/{n}.jpg` | Page previews for pages 2 and up |

Deleting a document deletes every object under `documents/{id}/`, plus any
files written by older versions.

## Local

```sh
FOLIO_STORAGE__BACKEND=local
FOLIO_STORAGE__PATH=/data/documents
```

The directory is created on start. Keys map to paths below it.

## S3 and compatible services

```sh
FOLIO_STORAGE__BACKEND=s3
FOLIO_STORAGE__S3__BUCKET=folio
FOLIO_STORAGE__S3__REGION=eu-central-1
```

Create the bucket yourself; Folio does not. Credentials come from
`FOLIO_STORAGE__S3__ACCESS_KEY_ID` and `FOLIO_STORAGE__S3__SECRET_ACCESS_KEY`
(use the `_FILE` form for secrets), or from the standard `AWS_*` environment
variables, web identity tokens and instance metadata when those are unset.

An optional `FOLIO_STORAGE__S3__PREFIX=folio/` scopes all keys to a prefix so
several installations can share one bucket.

### MinIO

```sh
FOLIO_STORAGE__BACKEND=s3
FOLIO_STORAGE__S3__ENDPOINT=http://minio:9000
FOLIO_STORAGE__S3__BUCKET=folio
FOLIO_STORAGE__S3__ACCESS_KEY_ID=minioadmin
FOLIO_STORAGE__S3__SECRET_ACCESS_KEY_FILE=/run/secrets/minio_secret
FOLIO_STORAGE__S3__PATH_STYLE=true
FOLIO_STORAGE__S3__ALLOW_HTTP=true
```

`docker-compose.example.yml` runs Folio with PostgreSQL and MinIO. Create the
bucket once before first use; Folio does not.

### Garage

```sh
FOLIO_STORAGE__S3__ENDPOINT=http://garage:3900
FOLIO_STORAGE__S3__REGION=garage
FOLIO_STORAGE__S3__PATH_STYLE=true
```

### Cloudflare R2

```sh
FOLIO_STORAGE__S3__ENDPOINT=https://<account-id>.r2.cloudflarestorage.com
FOLIO_STORAGE__S3__REGION=auto
```

## Moving from the old layout

Versions before the storage port wrote `{id}/{file name}` and
`thumbnails/{id}.jpg` under `FOLIO_STORAGE__PATH`. On start, Folio copies those
objects to their new keys, updates `documents.file_path`, and removes the old
ones. The pass is idempotent. Until a document is moved, its old path keeps
working for reads. Switching an existing installation to S3 requires copying
the old directory into the bucket first (for example with `rclone`), then
starting Folio so the pass can rename the keys.

## Working with local tools

OCR and thumbnail generation shell out to `pdftoppm` and `pdfinfo`, which need
files. Folio streams the object into a temporary file, runs the tool, and
deletes the file. Generated thumbnails and pages are written back through the
store.

## Downloads

Downloads and thumbnails are streamed from the store through Folio with the
stored MIME type, a `Content-Length` when the backend reports one, and an
RFC 6266 `Content-Disposition` carrying both an ASCII `filename` and a UTF-8
`filename*`. The object store never needs to be reachable by browsers.

## Tests

`cargo test -p folio-infra --test storage_contract` runs one contract against
the local, in-memory and S3 backends. The S3 case starts MinIO (Chainguard's `cgr.dev/chainguard/minio` image, because
`minio/minio` is no longer pulled from Docker Hub) with testcontainers and needs a Docker daemon; without one it prints `SKIPPED`
unless `FOLIO_REQUIRE_DOCKER=1` is set, as it is in CI.
