# apple-server-contract Specification

## Purpose
Defines what the Apple apps (the app, the File Provider extension, and Spotlight links) require from the server. Clients must complete vault discovery first, and they reject unscoped folder, document, and link identifiers instead of falling back to a default vault. Apple app users rely on this so that files and links always open in the vault they belong to.
## Requirements
### Requirement: Require vault discovery

Apple clients SHALL require a successful `GET /api/vaults` response with status 200 and a valid accessible default vault before exposing vault operations. A 404 response SHALL remain an error and SHALL NOT enable an older server mode.

#### Scenario: Server lacks vault discovery

- **WHEN** `GET /api/vaults` returns 404
- **THEN** the client reports the server error without exposing an unscoped library

#### Scenario: Discovery has not completed

- **WHEN** the catalog has not loaded successfully
- **THEN** the client grants no upload or editing capability based on an absent vault

### Requirement: Require scoped File Provider identity

File Provider SHALL expose vault-qualified folders and document identifiers. It SHALL reject unscoped identifiers without issuing document operations against a default vault.

#### Scenario: Old provider identifier

- **WHEN** an unscoped folder or document identifier is resolved
- **THEN** it cannot resolve or mutate a document through the default vault

#### Scenario: Discovery fails transiently

- **WHEN** catalog discovery fails
- **THEN** enumeration reports the failure rather than treating all previously visible contents as deleted

### Requirement: Require scoped external document identifiers

Apple clients SHALL generate and parse external Spotlight document identifiers containing both vault and document UUIDs. UUID-only identifiers and arbitrary prefixes followed by a UUID SHALL be rejected.

#### Scenario: Qualified document link

- **WHEN** a valid vault-qualified document identifier is opened
- **THEN** navigation resolves the document in that vault

#### Scenario: Old UUID-only link

- **WHEN** a UUID-only external identifier is opened
- **THEN** it is rejected without default-vault lookup

