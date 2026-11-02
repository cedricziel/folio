# Integration shortcuts and personal access tokens

## Why

API documentation and the MCP endpoint exist, but the web navigation does not help users find them. OAuth already supports bearer tokens and document scopes. Scripts need user-managed credentials without an interactive OAuth flow.

## What changes

- Add web navigation shortcuts to API docs and an MCP connection page.
- Let signed-in users create, list, and revoke personal access tokens with an explicit audience, scopes, and expiry.
- Reuse bearer validation and existing vault authorization. A token cannot grant permissions its owner lacks.
- Preserve OAuth clients, browser sessions, and existing anonymous deployments.

## Capabilities

### New capabilities

- `integration-shortcuts`: discover API docs and MCP connection details from the web app.
- `personal-access-tokens`: manage scoped, expiring credentials for the JSON API or MCP.

## Impact

Touches folio-auth models and AuthStore, folio-infra SQLite and PostgreSQL auth adapters, folio-api routes and templates, folio-mcp authentication, and folio-app wiring. Existing folio-core vault policy remains authoritative; no new core domain port is needed because AuthStore already owns credentials.

Adds HTML GET `/settings/integrations` and JSON GET/POST `/api/auth/tokens`, DELETE `/api/auth/tokens/{id}`. Management requires a signed-in account session, including when instance authentication is optional. Bearer tokens cannot manage tokens.

Add `20261102140000_personal_access_tokens.sql` in both migration directories, adjusting the timestamp if occupied before implementation. Existing rows and OAuth grants remain unchanged. No configuration setting is added.

Native token-management UI, vault-restricted tokens, new OAuth flows, and new administrative scopes are outside this change.
