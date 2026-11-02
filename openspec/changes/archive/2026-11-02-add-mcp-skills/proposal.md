## Why

MCP consumers can call Folio tools but cannot discover workflow guidance from the server. Serve a bundled archive skill so consumers can load instructions when needed.

## What changes

- Declare the official skills extension and implement skills/list and skills/get.
- Serve a static skill and supporting workflow reference through resources/read with byte sizes and SHA-256 manifests.
- Document consumer discovery, verification, loading, and fallback use.

## Impact

Changes stay in folio-mcp and documentation. No folio-core ports/models, folio-infra adapters, or folio-api routes/templates change. Existing MCP HTTP and stdio transports carry the methods. There are no new HTML or JSON API routes, settings, migrations, OpenAPI changes, or Swift changes.
