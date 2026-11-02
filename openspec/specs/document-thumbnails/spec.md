# document-thumbnails Specification

## Purpose

Provide small document previews without downloading original files or changing document access controls.

## Requirements

### Requirement: Authenticated thumbnail retrieval

`GET /api/documents/{id}/thumbnail` SHALL apply the same authentication policy, bearer-token `documents:read` scope, and selected-vault access as document retrieval. It SHALL return a generated thumbnail as `200 image/jpeg` with `Cache-Control: private, no-store`. It SHALL NOT substitute an original file or an SVG placeholder.

#### Scenario: Generated thumbnail exists

- **WHEN** an authorized reader requests an accessible document with a generated thumbnail
- **THEN** the response is 200 with the JPEG bytes and private, no-store cache policy

#### Scenario: Thumbnail or document unavailable

- **WHEN** the generated thumbnail is absent or the document is missing, trashed, or outside the selected vault
- **THEN** the response is 404 and no original file bytes are returned

#### Scenario: Authentication or scope fails

- **WHEN** authentication is required and the token is missing or invalid
- **THEN** the response is 401

#### Scenario: Token scope is insufficient

- **WHEN** a valid token lacks document read scope
- **THEN** the response is 403

#### Scenario: Storage fails

- **WHEN** the thumbnail store fails for a reason other than a missing object
- **THEN** the response is 500 rather than a successful placeholder

### Requirement: Bounded Apple thumbnail transport

Apple clients SHALL retrieve thumbnails through the authenticated, vault-scoped service. They SHALL treat 404 as unavailable, retain authorization errors, and reject malformed or oversized image responses before displaying them.

#### Scenario: Access token expires

- **WHEN** the thumbnail request receives 401 and token refresh succeeds
- **THEN** the client retries with the fresh token and returns the JPEG
- **AND** failed refresh remains an authentication error

#### Scenario: Unusable response

- **WHEN** a response is not a valid JPEG or exceeds the client byte or pixel limits
- **THEN** the client rejects it instead of displaying or retaining the bytes

### Requirement: File Provider thumbnail batches

File Provider SHALL retrieve previews for vault-qualified document identifiers and report each item's data, unavailable result, or error independently. Folders SHALL use the system fallback without a thumbnail request.

#### Scenario: Mixed batch

- **WHEN** a batch includes a readable thumbnail, an unavailable thumbnail, and a denied item
- **THEN** each item receives its own result and the batch completes once after those callbacks

#### Scenario: Batch cancellation

- **WHEN** the caller cancels the returned progress
- **THEN** pending network work is cancelled and completion reports cancellation once
- **AND** no item callbacks occur after completion

### Requirement: Nonblocking document row previews

Apple document rows SHALL show available previews in the existing thumbnail area without delaying document opening. Loading, unavailable, and failed previews SHALL retain the paper placeholder without an alert.

#### Scenario: Preview unavailable

- **WHEN** a thumbnail is missing or its request fails authentication
- **THEN** the row retains its placeholder and remains openable

### Requirement: Workspace-scoped memory cache

Thumbnail bytes SHALL be cached only in memory with a bounded byte budget. Cache ownership SHALL follow the signed-in workspace and selected vault; document revisions SHALL identify separate entries. Failed and unavailable results SHALL remain retryable.

#### Scenario: Vault or document changes

- **WHEN** the selected vault changes or the document revision changes
- **THEN** an earlier cached preview cannot satisfy the new request

#### Scenario: Concurrent callers cancel

- **WHEN** multiple rows request the same preview
- **THEN** they share one transfer
- **AND** cancelling the last caller cancels that transfer
