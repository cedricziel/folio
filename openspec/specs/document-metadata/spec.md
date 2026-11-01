# document-metadata Specification

## Purpose

Correspondents, document types and tags: the vocabularies used to classify documents, how they are managed, and how they are assigned to documents.

## Requirements

### Requirement: Classification vocabularies

The system SHALL keep three named vocabularies: correspondents (who sent a document), document types, and tags. Names SHALL be unique within each vocabulary. A tag MAY carry a display color.

#### Scenario: Duplicate name through the JSON API

- **WHEN** a client posts `{"name": "Bank"}` to `/api/tags` and a tag named `Bank` already exists
- **THEN** the response status is 409 and the body names the conflicting value

#### Scenario: Tag with a color

- **WHEN** a tag is created with color `#ff5733`
- **THEN** the tag is stored and listed with that color

### Requirement: Listing and creating through the JSON API

`GET /api/tags`, `GET /api/correspondents` and `GET /api/document_types` SHALL return the vocabulary as a JSON array ordered by name with status 200. `POST` to the same paths with a JSON `name` (and `color` for tags) SHALL create an entry and return it with status 200.

#### Scenario: List tags

- **WHEN** a client requests `GET /api/tags`
- **THEN** the response status is 200 and the tags are sorted by name

#### Scenario: Create a correspondent

- **WHEN** a client posts `{"name": "Utility Co"}` to `/api/correspondents`
- **THEN** the response status is 200 and the body is the new correspondent

### Requirement: Management page

The system SHALL provide a page at `/manage` to create, rename and delete correspondents, document types and tags. Each form SHALL post to `/manage/<vocabulary>/create`, `/manage/<vocabulary>/:id/update` or `/manage/<vocabulary>/:id/delete` and receive the updated list as an HTML fragment.

#### Scenario: Rename a document type

- **WHEN** a user renames the document type `Invoice` to `Invoices` on `/manage`
- **THEN** the returned fragment lists `Invoices`

#### Scenario: Clear a tag color

- **WHEN** a user saves a tag with an empty color field
- **THEN** the tag no longer has a color

### Requirement: Deleting a vocabulary entry

Deleting a correspondent or document type SHALL leave its documents in place with that field cleared. Deleting a tag SHALL remove it from every document.

#### Scenario: Delete a correspondent in use

- **WHEN** a correspondent assigned to two documents is deleted
- **THEN** both documents remain and have no correspondent

### Requirement: Assigning metadata to a document

The document page SHALL set a correspondent at `POST /documents/:id/correspondent`, set a document type at `POST /documents/:id/document_type`, add a tag at `POST /documents/:id/tags`, and remove a tag at `POST /documents/:id/tags/:tag_id`. Each SHALL redirect to `/documents/:id`.

#### Scenario: Assign a correspondent

- **WHEN** a user submits a correspondent ID for a document
- **THEN** the document's correspondent is set and the browser is redirected to the document page

#### Scenario: Clear the correspondent

- **WHEN** a user submits an empty correspondent ID
- **THEN** the document's correspondent is cleared

#### Scenario: Add and remove a tag

- **WHEN** a user adds the tag `Tax` to a document and then removes it
- **THEN** the document has no `Tax` tag afterwards
