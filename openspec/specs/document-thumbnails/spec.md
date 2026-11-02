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
