## 1. Remove Apple compatibility paths

- [x] Verify `missingVaultDiscoveryIsAnError` fails before removing the older-server fallback.
- [x] Cover rejected unscoped provider identifiers and require accessible vault context.
- [x] Verify `unscopedIdentifiersAreRejected` fails before requiring scoped Spotlight identity.
- [x] Update current client fixtures and producers to use explicit vaults.

## 2. Verify and ship

- [x] Run isolated Swift package and native authentication/provider tests, plus Swift formatting checks.
- [x] Update the project verification skill and validate this change with OpenSpec.
- [x] Run make format, make lint, and make test.
- [x] Review the diff and archive this verified change.

CI and merge are tracked by the pull request.
