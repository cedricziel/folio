## ADDED Requirements

### Requirement: Session-owned token lifecycle

The server SHALL expose session-only personal-token management with explicit audience, scopes, and finite expiry.

#### Scenario: Create a token
- **WHEN** an authenticated same-origin session sends valid name, audience, scopes, and expiry to POST `/api/auth/tokens`
- **THEN** the server returns 201 with metadata and a one-time secret and stores only its hash

#### Scenario: Reject invalid permissions
- **WHEN** POST `/api/auth/tokens` receives empty or unknown scopes, an invalid audience, name, or expiry
- **THEN** the server returns 400 without creating a token

#### Scenario: List own tokens
- **WHEN** a signed-in user requests GET `/api/auth/tokens`
- **THEN** the server returns 200 with only that user's metadata and no secrets or hashes

#### Scenario: Revoke own token
- **WHEN** a signed-in user sends DELETE `/api/auth/tokens/{id}` for their token
- **THEN** the server returns 204 and subsequent authenticated requests using that token fail

#### Scenario: Protect other users
- **WHEN** a signed-in user sends DELETE `/api/auth/tokens/{id}` for another user's token or an unknown id
- **THEN** the server returns 404 without changing credentials

#### Scenario: Reject bearer management and cross-origin mutations
- **WHEN** a caller without a session uses a bearer token on GET or POST `/api/auth/tokens` or DELETE `/api/auth/tokens/{id}`
- **THEN** the server returns 401
- **WHEN** a session sends a cross-origin mutation to those endpoints
- **THEN** the server returns 403

### Requirement: Scoped resource access

Personal tokens SHALL authenticate only their selected resource and SHALL intersect document scopes with the owner's current vault permissions.

#### Scenario: Read archive data
- **WHEN** an API token with documents:read requests GET `/api/documents` or POST `/api/documents/search` in an accessible vault
- **THEN** the server returns 200 subject to the existing query contract

#### Scenario: Deny write without scope
- **WHEN** a read-only API token sends POST `/api/documents/upload`
- **THEN** the server returns 403 with an insufficient_scope bearer challenge before storing a file

#### Scenario: Reject invalid credentials without cookie fallback
- **WHEN** GET `/api/documents` carries an expired, revoked, invalid, wrong-audience, or deleted-owner token, even alongside a valid session cookie
- **THEN** the server returns 401 with a bearer challenge

#### Scenario: Preserve vault isolation
- **WHEN** a valid token requests GET `/api/documents` with an inaccessible X-Folio-Vault
- **THEN** the server returns 404
- **WHEN** the owner has access but lacks the role required for DELETE `/api/documents/{id}`
- **THEN** the server returns 403 even with documents:write

#### Scenario: Authenticate MCP
- **WHEN** a personal token targeting MCP sends POST `/mcp`
- **THEN** the transport authenticates the owner and tools enforce the token's explicit scopes and current vault permissions
- **WHEN** a revoked token reuses an existing MCP transport session
- **THEN** the next POST `/mcp` returns 401

#### Scenario: Preserve existing clients
- **WHEN** existing OAuth clients or browser sessions access their authorized resources
- **THEN** their audience, refresh, and vault behavior remains compatible
- **WHEN** authentication is optional and a request carries a personal token
- **THEN** its scopes and validity are enforced while credential-free requests retain existing behavior

### Requirement: MCP session ownership

Stateful MCP transport sessions SHALL remain bound to the authenticated account, client or personal token, scopes, and current vault access.

#### Scenario: Reject another caller's session
- **WHEN** a different authenticated caller sends POST, GET, or DELETE `/mcp` using an existing session identifier
- **THEN** the server returns 404 without replaying output, executing a tool, or deleting the session

#### Scenario: Recheck access before replay
- **WHEN** a caller's vault membership or role changes after session initialization
- **THEN** GET `/mcp` for that session returns 404 rather than replaying data under outdated permissions

#### Scenario: Preserve OAuth refresh
- **WHEN** the same OAuth account and client use a replacement access token with unchanged audience, scopes, and vault access
- **THEN** their existing MCP session remains usable
