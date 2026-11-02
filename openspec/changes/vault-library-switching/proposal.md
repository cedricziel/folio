# Vault selection in the library

## Why

Vault isolation exists, but clients cannot discover or select an accessible vault. The approved design places vault selection above the existing library navigation on web, macOS, iPadOS, and iOS.

## What changes

- Add JSON discovery at `GET /api/vaults` and request selection through `X-Folio-Vault` while preserving existing default-vault API calls.
- Add explicit HTML navigation under `/vaults/{vault_id}`. Each page and window retains its own scope; switching does not change the account default.
- Enforce vault membership and action permissions before selected requests reach handlers.
- Expose stable, independently scoped vault folders through the Apple File Provider.
- Add native library and upload destination pickers. An upload captures its destination without switching the library.

This slice touches the core vault service, API routing and templates, OpenAPI contract, and Apple clients. It adds no database migration or configuration setting. Vault creation, membership administration, moves, and MCP selection remain in `add-vaults-and-access`.

The existing vault plan's removal of unscoped API paths is deferred. Keeping those paths prevents breaking deployed Apple and File Provider clients during rollout.
