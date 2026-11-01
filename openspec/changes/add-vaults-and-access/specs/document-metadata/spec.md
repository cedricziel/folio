# Spec Delta

## MODIFIED Requirements

### Requirement: Classification vocabularies

The system SHALL keep three named vocabularies per vault: correspondents (who sent a document), document types, and tags. Names SHALL be unique within each vocabulary of a vault; two vaults MAY use the same name. A tag MAY carry a display color.

#### Scenario: Duplicate name through the JSON API

- **WHEN** a client posts `{"name": "Bank"}` to `/api/vaults/{home}/tags` and `Home` already has a tag named `Bank`
- **THEN** the response status is 409 and the body names the conflicting value

#### Scenario: Same name in another vault

- **WHEN** a client posts `{"name": "Bank"}` to `/api/vaults/{business}/tags` and only `Home` has a tag named `Bank`
- **THEN** the response status is 200 and `Business` has its own tag `Bank`

#### Scenario: Tag with a color

- **WHEN** a tag is created with color `#ff5733`
- **THEN** the tag is stored and listed with that color

### Requirement: Listing and creating through the JSON API

`GET /api/vaults/{vault_id}/tags`, `.../correspondents` and `.../document_types` SHALL return the vault's vocabulary as a JSON array ordered by name with status 200 to any member of the vault. `POST` to the same paths with a JSON `name` (and `color` for tags) SHALL create an entry in the vault and return it with status 200, for editors and owners only.

#### Scenario: List tags

- **WHEN** a viewer of `Home` requests `GET /api/vaults/{home}/tags`
- **THEN** the response status is 200 and only `Home` tags are returned, sorted by name

#### Scenario: Create a correspondent

- **WHEN** an editor of `Home` posts `{"name": "Utility Co"}` to `/api/vaults/{home}/correspondents`
- **THEN** the response status is 200 and the body is the new correspondent

#### Scenario: Viewer creates a tag

- **WHEN** a viewer of `Home` posts `{"name": "Tax"}` to `/api/vaults/{home}/tags`
- **THEN** the response status is 403

#### Scenario: Vault without a role

- **WHEN** a user with no role in `Home` requests `GET /api/vaults/{home}/tags`
- **THEN** the response status is 404

#### Scenario: Former unscoped path

- **WHEN** a client requests `GET /api/tags`
- **THEN** the response status is 404

### Requirement: Management page

The system SHALL provide a page at `/vaults/{vault_id}/manage` for editors and owners to create, rename and delete the vault's correspondents, document types and tags. Each form SHALL post to `/vaults/{vault_id}/manage/<vocabulary>/create`, `.../:id/update` or `.../:id/delete` and receive the updated list as an HTML fragment.

#### Scenario: Rename a document type

- **WHEN** an editor of `Home` renames the document type `Invoice` to `Invoices` on `/vaults/{home}/manage`
- **THEN** the returned fragment lists `Invoices`

#### Scenario: Clear a tag color

- **WHEN** an editor saves a tag with an empty color field
- **THEN** the tag no longer has a color

#### Scenario: Contributor opens the management page

- **WHEN** a contributor of `Home` opens `/vaults/{home}/manage`
- **THEN** the response status is 403

### Requirement: Assigning metadata to a document

The document page SHALL set a correspondent at `POST /documents/:id/correspondent`, set a document type at `POST /documents/:id/document_type`, add a tag at `POST /documents/:id/tags`, and remove a tag at `POST /documents/:id/tags/:tag_id`. Each SHALL redirect to `/documents/:id`. Only editors and owners, or the uploader with the contributor role, SHALL assign metadata, and only entries from the document's vault.

#### Scenario: Assign a correspondent

- **WHEN** an editor submits a correspondent ID from the document's vault
- **THEN** the document's correspondent is set and the browser is redirected to the document page

#### Scenario: Clear the correspondent

- **WHEN** an editor submits an empty correspondent ID
- **THEN** the document's correspondent is cleared

#### Scenario: Add and remove a tag

- **WHEN** an editor adds the tag `Tax` to a document and then removes it
- **THEN** the document has no `Tax` tag afterwards

#### Scenario: Contributor tags someone else's document

- **WHEN** a contributor adds a tag to a document another user uploaded
- **THEN** the response status is 403 and the document's tags are unchanged

#### Scenario: Correspondent from another vault

- **WHEN** an editor submits the ID of a correspondent from a different vault
- **THEN** the response status is 404 and the document's correspondent is unchanged

## ADDED Requirements

### Requirement: Classification follows a moved document

When a document moves to another vault, the system SHALL replace its correspondent, document type and tags with the entries of the same name in the target vault, creating entries that do not exist there yet (with the tag's color).

#### Scenario: Matching tag exists

- **WHEN** a document tagged `Tax` moves from `Home` to `Business`, and `Business` has a tag `Tax`
- **THEN** the document carries the `Business` tag `Tax`

#### Scenario: Correspondent missing in the target

- **WHEN** a document with correspondent `Utility Co` moves to `Business`, which has no such correspondent
- **THEN** `Business` gains a correspondent `Utility Co` and the document references it
- **AND** the `Home` correspondent `Utility Co` still exists
