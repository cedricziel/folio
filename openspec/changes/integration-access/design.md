# Design

## Existing behavior

`crates/folio-api/src/openapi.rs` serves `/api/docs` and `/api/openapi.json`. `folio-auth/src/validator.rs` validates opaque OAuth tokens against the configured resource audience. `folio-api/src/routes/auth.rs::enforce_auth` accepts sessions or API bearer tokens and checks document scopes. MCP bridges AuthContext to McpAuth and checks scopes per tool. Vault authorization remains an additional check.

## Credentials

Add a distinct personal-token record to AuthStore and both database implementations. Store id, owner user id, name, token hash, non-secret display prefix, audience, scopes, created_at, expires_at, and revoked_at. Cascade on user deletion. Generate at least 256 random bits, with a recognizable `folio_pat_` prefix. Store only the existing cryptographic token hash. Return the raw token once; never log it or persist it in browser storage.

Creation accepts name of 1–64 trimmed characters, audience `api` or `mcp`, a nonempty explicit subset of `documents:read` and `documents:write`, and expiry of 1–365 days. UI defaults to read-only and 30 days. Write does not imply read. No wildcard or implicit scope expansion. Bind each token to the configured public base URL plus `/api` or `/mcp`, never request Host headers. One token targets one resource.

GET returns metadata only. POST returns metadata plus `token`. DELETE records revocation immediately. Responses use Cache-Control: no-store. Use existing same-origin session mutation protection. Personal tokens cannot issue or revoke other credentials, change account security, or access instance administration.

## Enforcement

Extend TokenValidator to resolve personal tokens without changing OAuth audience or refresh semantics. Reject expired, revoked, unknown, wrong-audience, and deleted-owner credentials. Explicit Authorization headers take precedence over ambient cookies: invalid bearer credentials cannot fall back to a browser session. Apply this rule to protected API routes while preserving public docs and health exemptions.

Keep the existing two document scopes. Classify API actions by behavior, including POST document search as read, rather than relying solely on HTTP method. Document writes require documents:write. Vocabulary reads and writes follow their respective document scope. Existing role checks still restrict workflows and sources. Token management and account/admin endpoints are session-only for personal tokens. Enumerate protected routes during implementation and deny personal-token access to unclassified routes. OAuth compatibility must have regression coverage.

Anonymous deployment mode stays unchanged for requests without credentials. Explicit personal credentials must still be validated and scoped when supplied, even if auth.required is false. MCP requests must retain the authenticated user and scopes through transport sessions; revocation and changed vault membership take effect on the next request.

## Web experience

Add `API docs` and `MCP server` links in the shared navigation, outside vault-specific URLs. API docs opens `/api/docs`. MCP server opens `/settings/integrations`, which explains OAuth and shows a copyable configured absolute `/mcp` URL. A disabled MCP server is labelled disabled, using the same effective setting precedence as the transport, without enabling it automatically.

The integrations page contains token creation, metadata listing, and revoke controls. Display the secret once with a copy control and a clear reminder to save it. Subsequent renders cannot retrieve it. Use existing design tokens, forms, icons, responsive navigation, and accessibility conventions. Anonymous users see a sign-in requirement for token management. Never append credentials to links or examples.

## Contract and rollout

Document management endpoints, session-only security, bearer authentication, audiences, scopes, and 400/401/403/404 outcomes in OpenAPI. Regenerate `openapi/openapi.json` and the existing generated Swift client using repository commands; no Apple UI changes. Add migrations without editing old migrations. Test SQLite and PostgreSQL persistence contracts when their isolated test infrastructure is available.

## Work ownership

The auth worker owns folio-auth, folio-infra, migrations, middleware in routes/auth.rs, a new routes/tokens.rs, and folio-app MCP wiring. The UI worker owns navigation, integration templates, a new routes/integrations.rs, and UI tests. The coordinating agent owns shared route registration, OpenAPI generation, final integration, and verification. Workers agree on the JSON contract above and do not edit each other's files.
