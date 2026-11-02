## ADDED Requirements

### Requirement: Discover accessible vaults

`GET /api/vaults` SHALL return 200 with `vaults` containing each accessible vault's `id`, `name`, and effective `role`, and `default_vault_id`. It SHALL omit inaccessible vaults.

#### Scenario: Discover own and shared vaults
- **WHEN** a signed-in user requests `GET /api/vaults`
- **THEN** the response includes direct and group memberships, with the highest effective role, and excludes other users' private vaults

### Requirement: Select a vault per request

JSON API requests SHALL accept `X-Folio-Vault` containing a UUID. Existing requests without it SHALL use the caller's default vault. HTML routes under `/vaults/{vault_id}` SHALL use that vault. Selection SHALL NOT change the stored default.

#### Scenario: Isolation between selected vaults
- **WHEN** a caller selects a vault they can read
- **THEN** lists, searches, vocabularies, document operations, and events use that vault only

#### Scenario: Unknown or inaccessible vault
- **WHEN** a request selects an unknown vault or a vault with no effective membership
- **THEN** the response is 404 with the same error body

#### Scenario: Malformed selection
- **WHEN** the header or URL contains an invalid vault UUID
- **THEN** the response is 400

#### Scenario: Revoked membership
- **WHEN** membership is revoked after a page loads
- **THEN** the next selected request returns 404

### Requirement: Enforce selected vault roles

The server SHALL enforce the existing `VaultRole` action matrix on scoped and default requests. Permission failures SHALL return 403. Viewers SHALL NOT create, modify, or delete documents. Contributors SHALL modify only documents they uploaded. Vocabulary management SHALL require editor or owner, and workflow management SHALL require owner.

#### Scenario: Viewer tries to upload
- **WHEN** a viewer posts an upload to a selected vault
- **THEN** the server returns 403 before importing the file

### Requirement: Vault selection in library navigation

Web and Apple clients SHALL show the selected vault above library destinations. A single accessible vault SHALL display its name without a redundant selector. Changing vaults SHALL clear stale document selection and queries and scope subsequent operations to the selected vault.

#### Scenario: Switch while a request is pending
- **WHEN** a user switches vaults while the previous library request is pending
- **THEN** its late result cannot populate the newly selected library

### Requirement: Independent upload destination

Upload UI SHALL offer writable vaults and capture the selected destination when upload begins. Changing an upload destination SHALL NOT change the library vault. Background transfers SHALL retain the captured destination.

#### Scenario: File into another vault
- **WHEN** a user browses one vault and uploads into another
- **THEN** the file and its selected tags belong to the upload destination and the library remains on the original vault

### Requirement: Stable vault folders in Files and Finder

The Apple File Provider SHALL list accessible vaults as top-level folders, each containing its own library folders. Folder and document-alias identifiers SHALL include the vault UUID and remain independent of app selection and vault renames. Duplicate names SHALL be disambiguated consistently.

#### Scenario: Browse while the app switches vaults
- **WHEN** Files or Finder opens a document in one vault and the app switches to another
- **THEN** the provider continues to resolve and download the original document in its original vault

#### Scenario: Upload into a vault inbox
- **WHEN** a contributor, editor, or owner creates a file in a vault's Inbox
- **THEN** the provider captures that vault as the upload destination
- **AND** the provider root and viewer inboxes do not advertise file creation

#### Scenario: Membership or role changes
- **WHEN** access to a vault is revoked or downgraded
- **THEN** the next provider enumeration removes inaccessible items or updates capabilities
- **AND** inaccessible materialization returns no-such-item without falling back to another vault

#### Scenario: Transient enumeration failure
- **WHEN** a vault request fails due to a network or server error
- **THEN** the provider reports an error without treating its contents as deleted

#### Scenario: Existing unscoped identifiers
- **WHEN** a vault-capable server is used with a previously unscoped provider identifier
- **THEN** the provider rejects that identifier instead of silently rebinding it to a different vault
