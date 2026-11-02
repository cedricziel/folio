## 1. Credential storage and authentication

- [ ] 1.1 Write failing token-store lifecycle and owner-isolation tests, then add personal-token models, AuthStore methods, paired migrations, and both adapters. Commit as feat(auth): store personal access tokens.
- [ ] 1.2 Write failing validator tests for expiry, revocation, audience, deleted owner, and explicit scopes, then implement personal-token validation with OAuth regression coverage. Commit as feat(auth): validate scoped personal tokens.
- [ ] 1.3 Write failing management endpoint tests for create/list/revoke, session-only access, cross-origin rejection, and secret redaction, then implement JSON handlers. Commit as feat(api): manage personal access tokens.
- [ ] 1.4 Write failing API/MCP tests for cookie precedence, optional-auth credentials, POST search, denied writes, vault restrictions, and MCP session revocation, then integrate enforcement. Commit as fix(auth): enforce personal token permissions.

## 2. Integration web page

- [ ] 2.1 Write failing web tests for desktop/mobile shortcuts, vault-independent links, and effective MCP disabled state, then add navigation and integration page. Commit as feat(web): expose API and MCP shortcuts.
- [ ] 2.2 Write failing UI tests for create-once secret display, copy, list, and revoke using the agreed API contract, then add token controls. Commit as feat(web): manage integration tokens.

## 3. Integration and verification

- [ ] 3.1 Register new routes, write failing OpenAPI contract tests, update security definitions, and regenerate OpenAPI and Swift artifacts. Commit as docs(api): document personal token authentication.
- [ ] 3.2 Run the project-local verify skill with isolated state and update its covered behavior. Validate this OpenSpec change. Record unavailable infrastructure and remaining checks accurately.
- [ ] 3.3 Run make format, make lint, and make test. Review security boundaries and all worker diffs before reporting completion.
