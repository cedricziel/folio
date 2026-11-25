# Domain lifecycle

## Why

Persisted objects expose inconsistent ownership and attribution. Classification deletion destroys references, while documents already support trash. New domain features need one lifecycle contract.

## What changes

Add consistent lifecycle metadata and explicit vault ownership to mutable domain models. Preserve immutable events and completed history. Attribute writes to authenticated users or an explicit unknown/system actor. Add deletion, restoration, and history controls to the web and Apple interfaces.

## Capabilities

- New `domain-lifecycle`: attribution, retained deletion, restoration, and ownership.

## Impact

- `folio-core`: models, repository contracts, mutation context.
- `folio-infra`: SQLite and PostgreSQL adapters and new `domain_lifecycle` migrations. Preserve existing timestamps; backfill missing update times from creation times and leave unknown actors null.
- `folio-api`: HTML Turbo and JSON lifecycle routes, authenticated attribution, and OpenAPI.
- `folio-mcp`: authenticated write attribution and compatible response additions.
- Apple: generated client, metadata display, deletion and restoration controls.
- No new settings or configuration precedence.
