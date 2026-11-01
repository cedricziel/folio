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

The system SHALL store each uploaded file at `<storage path>/<document id>/<sanitized filename>` and record that relative path on the document. Sanitizing SHALL drop directory components and replace every character other than letters, digits, `.`, `-` and `_` with `_`.

#### Scenario: Filename with spaces and directories

- **WHEN** a file named `../scans/my letter.pdf` is uploaded
- **THEN** it is stored as `<document id>/my_letter.pdf`

#### Scenario: Filename that starts with a dot

- **WHEN** a file named `.hidden` is uploaded
- **THEN** the stored filename is prefixed with `document_`

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

The system SHALL delete a document at `POST /documents/:id/delete` and respond with a Turbo Stream. Deleting a document SHALL also remove its tag assignments and stored embedding.

#### Scenario: Delete an existing document

- **WHEN** a user deletes an existing document
- **THEN** the document no longer appears in listings or search results

#### Scenario: Delete a missing document

- **WHEN** a user deletes a document ID that does not exist
- **THEN** the response status is 404
