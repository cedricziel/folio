# document-upload Specification

## Purpose

How documents enter Folio, where their files are stored, and how they are listed and deleted. Text extraction and thumbnails are covered by document-processing.

## Requirements

### Requirement: Web upload queues processing

The system SHALL accept a multipart upload at `POST /documents/upload` with a required `file` part and an optional `title` part. It SHALL create the document without content, store the file, enqueue a document-processing job, and respond with a Turbo Stream (`text/vnd.turbo-stream.html`).

#### Scenario: Upload with a title

- **WHEN** a user uploads `invoice.pdf` with the title `March invoice`
- **THEN** a document titled `March invoice` is created with MIME type taken from the file part
- **AND** a processing job for the document is enqueued
- **AND** the response is a Turbo Stream that updates the document list

#### Scenario: Upload without a title

- **WHEN** a user uploads `invoice.pdf` with an empty or missing `title` part
- **THEN** the document title is `invoice.pdf`

#### Scenario: Upload without a file

- **WHEN** the multipart request has no `file` part
- **THEN** the response status is 400

#### Scenario: Missing content type

- **WHEN** the file part carries no content type
- **THEN** the document MIME type is `application/octet-stream`

### Requirement: Files are stored per document

The system SHALL store each uploaded file in the configured object store (`local` directory or S3-compatible bucket) under the key `documents/<document id>/original` and record that key in the document's `file_path`. Thumbnails SHALL be stored as `documents/<document id>/thumb.jpg` and page previews as `documents/<document id>/pages/<n>.jpg`. Keys are opaque; no filesystem path is stored. At startup the system SHALL move objects written under the earlier layout (`<document id>/<file name>`, `thumbnails/<id>.jpg`) to their new keys and keep reading old rows until then.

#### Scenario: Upload with a filename

- **WHEN** a file named `../scans/my letter.pdf` is uploaded
- **THEN** it is stored under `documents/<document id>/original`
- **AND** the original name is kept in `original_filename`

#### Scenario: Row from the earlier layout

- **WHEN** a document's `file_path` is `<document id>/my_letter.pdf` and the object exists under that key
- **THEN** downloads still succeed
- **AND** the next start moves the object to `documents/<document id>/original` and updates `file_path`

### Requirement: Downloads stream from storage

`GET /api/documents/:id/download` SHALL stream the stored file with the document's MIME type, a `Content-Length` when the backend reports the size, and a `Content-Disposition: attachment` header with an ASCII `filename` and an RFC 6266 UTF-8 `filename*`. Thumbnail routes SHALL stream the stored JPEG with `Content-Type: image/jpeg`.

#### Scenario: Download with a non-ASCII filename

- **WHEN** a document with the original filename `Größe.pdf` is downloaded
- **THEN** the response carries `filename*=UTF-8''Gr%C3%B6%C3%9Fe.pdf`

### Requirement: JSON upload extracts text inline

The system SHALL accept the same multipart fields at `POST /api/documents/upload`, run OCR and embedding generation before responding, and return the created document as JSON with status 200. It SHALL NOT enqueue a processing job.

#### Scenario: Upload with a configured provider

- **WHEN** a file is uploaded and OCR returns text
- **THEN** the returned document's `content` holds that text
- **AND** an embedding for the text is stored for search

#### Scenario: Upload with the local provider

- **WHEN** a file is uploaded while the AI provider is `local`
- **THEN** the document is created with empty content and no embedding is stored

#### Scenario: Upload when OCR fails

- **WHEN** OCR returns an error, for example because the selected provider has no API key
- **THEN** the document is still created, with no content, and the response status is 200

### Requirement: Documents can be created without a file

The system SHALL create a document from JSON at `POST /api/documents` with a required `title` and optional `content`, `correspondent_id` and `document_type_id`, and return it with status 200.

#### Scenario: Create a text-only document

- **WHEN** a client posts `{"title": "Note", "content": "hello"}` to `/api/documents`
- **THEN** the response status is 200 and the body is the new document with no file path

### Requirement: Document listing

The system SHALL list documents newest first. `GET /api/documents` SHALL return `{"documents": [...], "count": n}` and accept optional `correspondent_id` and `document_type_id` query filters; when both are given, only the correspondent filter applies.

#### Scenario: List all documents

- **WHEN** a client requests `GET /api/documents`
- **THEN** the response status is 200 and documents are ordered by creation time, newest first

#### Scenario: Filter by correspondent

- **WHEN** a client requests `GET /api/documents?correspondent_id=<id>`
- **THEN** only documents with that correspondent are returned

#### Scenario: Filter on the home page

- **WHEN** a user opens `/` with a `correspondent_id` or `document_type_id` query parameter
- **THEN** the page lists only matching documents
- **AND** empty or malformed IDs are ignored

#### Scenario: Several filters on the home page

- **WHEN** a user opens `/` with both `correspondent_id` and `document_type_id`
- **THEN** only the correspondent filter applies

#### Scenario: Tag filter on the home page

- **WHEN** a user opens `/` with a `tag_id` query parameter
- **THEN** the parameter is accepted but ignored, and all documents are listed

### Requirement: Deleting a document

The system SHALL delete a document at `POST /documents/:id/delete` and respond with a Turbo Stream. Deleting a document SHALL also remove its tag assignments, its stored embedding and every stored object of the document, including thumbnails, page previews and files from the earlier layout.

#### Scenario: Delete an existing document

- **WHEN** a user deletes an existing document
- **THEN** the document no longer appears in listings or search results

#### Scenario: Delete a missing document

- **WHEN** a user deletes a document ID that does not exist
- **THEN** the response status is 404
