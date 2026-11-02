# Spec Delta

## RENAMED Requirements

- FROM: `### Requirement: JSON upload extracts text inline`
- TO: `### Requirement: JSON upload queues processing`

## MODIFIED Requirements

### Requirement: Web upload queues processing

The system SHALL accept a multipart upload at `POST /vaults/{vault_id}/documents/upload` with a required `file` part and an optional `title` part from a contributor, editor or owner of the vault. It SHALL create the document in that vault without content, record the caller as its uploader, store the file, enqueue a document-processing job, and respond with a Turbo Stream (`text/vnd.turbo-stream.html`).

#### Scenario: Upload with a title

- **WHEN** a contributor of `Home` uploads `invoice.pdf` with the title `March invoice`
- **THEN** a document titled `March invoice` is created in `Home` with MIME type taken from the file part and the contributor as uploader
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

#### Scenario: Viewer uploads

- **WHEN** a viewer of `Home` posts a file to `/vaults/{home}/documents/upload`
- **THEN** the response status is 403 and no document is created

#### Scenario: Upload to a vault without a role

- **WHEN** a user with no role in `Home` posts a file to `/vaults/{home}/documents/upload`
- **THEN** the response status is 404 and no document is created

### Requirement: Files are stored per document

The system SHALL store each uploaded file under the opaque storage key `vaults/<vault id>/documents/<document id>/original` and record that key on the document. Thumbnails SHALL be stored as `vaults/<vault id>/documents/<document id>/thumb.jpg` and page previews as `vaults/<vault id>/documents/<document id>/pages/<n>.jpg`. The original filename SHALL remain metadata and SHALL NOT control the storage path. Moving a document SHALL move its file and thumbnails to the target vault's prefix.

#### Scenario: Filename with spaces and directories

- **WHEN** a file named `../scans/my letter.pdf` is uploaded to vault `<v>`
- **THEN** it is stored as `vaults/<v>/documents/<document id>/original`
- **AND** the original name is kept in `original_filename`

#### Scenario: Filename that starts with a dot

- **WHEN** a file named `.hidden` is uploaded to vault `<v>`
- **THEN** it is stored as `vaults/<v>/documents/<document id>/original`
- **AND** `.hidden` is kept in `original_filename`

#### Scenario: File of a moved document

- **WHEN** a document stored under `vaults/<home>/documents/<id>/` is moved to vault `<business>`
- **THEN** its file and thumbnails are stored under `vaults/<business>/documents/<id>/` and nothing remains under the old prefix

#### Scenario: File stored before the upgrade

- **WHEN** a document uploaded before the upgrade still has the key `<document id>/<filename>`
- **THEN** its file is read and downloaded from that recorded key

### Requirement: JSON upload queues processing

The system SHALL accept the same multipart fields at `POST /api/vaults/{vault_id}/documents/upload` from a contributor, editor or owner of the vault. It SHALL record the caller as uploader, store the file through the shared importer, enqueue a document-processing job, and return the created document, including `vault_id`, as JSON with status 200. OCR and embedding generation SHALL run in the background.

#### Scenario: Upload with a configured provider

- **WHEN** a file is uploaded with a configured OCR provider
- **THEN** the response status is 200 and the returned document has no extracted content yet
- **AND** a processing job for that document is enqueued
- **AND** processing stores extracted text and embeddings in the document's vault

#### Scenario: Upload with OCR disabled

- **WHEN** a file is uploaded while the OCR provider is `none`
- **THEN** the document and its processing job are created
- **AND** processing completes without provider-generated text

#### Scenario: Upload when OCR fails

- **WHEN** background OCR fails after an upload
- **THEN** the uploaded document and file remain available
- **AND** the document records the failed OCR stage

#### Scenario: Upload to a vault without a role

- **WHEN** a client with no role in `Home` posts to `/api/vaults/{home}/documents/upload`
- **THEN** the response status is 404

#### Scenario: Former unscoped path

- **WHEN** a client posts to `/api/documents/upload`
- **THEN** the response status is 404

### Requirement: Documents can be created without a file

The system SHALL create a document from JSON at `POST /api/vaults/{vault_id}/documents` with a required `title` and optional `content`, `correspondent_id` and `document_type_id`, for a contributor, editor or owner of the vault, and return it with status 200. The correspondent and document type SHALL belong to the same vault.

#### Scenario: Create a text-only document

- **WHEN** a contributor of `Home` posts `{"title": "Note", "content": "hello"}` to `/api/vaults/{home}/documents`
- **THEN** the response status is 200 and the body is the new document in `Home` with no file path

#### Scenario: Correspondent from another vault

- **WHEN** a client posts a `correspondent_id` that belongs to `Business` to `/api/vaults/{home}/documents`
- **THEN** the response status is 404 and no document is created

### Requirement: Document listing

The system SHALL list the documents of one vault, newest first, to any member of the vault. `GET /api/vaults/{vault_id}/documents` SHALL return `{"documents": [...], "count": n}` and accept optional `correspondent_id` and `document_type_id` query filters; when both are given, only the correspondent filter applies. The HTML list (the home page of a vault) SHALL be served at `/vaults/{vault_id}`.

#### Scenario: List all documents

- **WHEN** a viewer of `Home` requests `GET /api/vaults/{home}/documents`
- **THEN** the response status is 200 and only `Home` documents are returned, ordered by creation time, newest first

#### Scenario: Filter by correspondent

- **WHEN** a client requests `GET /api/vaults/{home}/documents?correspondent_id=<id>`
- **THEN** only `Home` documents with that correspondent are returned

#### Scenario: Filter on the home page

- **WHEN** a user opens `/vaults/{home}` with a `correspondent_id` or `document_type_id` query parameter
- **THEN** the page lists only matching documents
- **AND** empty or malformed IDs are ignored

#### Scenario: Several filters on the home page

- **WHEN** a user opens `/vaults/{home}` with both `correspondent_id` and `document_type_id`
- **THEN** only the correspondent filter applies

#### Scenario: Tag filter on the home page

- **WHEN** a user opens `/vaults/{home}` with a `tag_id` query parameter
- **THEN** the parameter is accepted but ignored, and all documents of the vault are listed

#### Scenario: Vault without a role

- **WHEN** a user with no role in `Home` requests `GET /api/vaults/{home}/documents` or opens `/vaults/{home}`
- **THEN** the response status is 404

### Requirement: Deleting a document

The system SHALL delete a document at `POST /documents/:id/delete`, responding with a Turbo Stream, and at `DELETE /api/documents/{id}`, responding 204, when the caller is editor or owner of the document's vault, or its uploader with the contributor role. Deleting a document SHALL also remove its tag assignments, stored embedding, file and thumbnails.

#### Scenario: Delete an existing document

- **WHEN** an editor deletes an existing document
- **THEN** the document no longer appears in listings or search results

#### Scenario: Delete a missing document

- **WHEN** a user deletes a document ID that does not exist
- **THEN** the response status is 404

#### Scenario: Delete a document in a vault without a role

- **WHEN** a user with no role in the document's vault deletes it
- **THEN** the response status is 404 and the document still exists

#### Scenario: Viewer deletes

- **WHEN** a viewer of the document's vault deletes it
- **THEN** the response status is 403 and the document still exists
