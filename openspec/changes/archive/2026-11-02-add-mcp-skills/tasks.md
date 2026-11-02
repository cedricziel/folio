## 1. Implementation

- [x] 1.1 Write and observe failing skills discovery, lookup, manifest, and invalid-URI integration tests.
- [x] 1.2 Implement the embedded catalog, request handlers, and resource reads.
- [x] 1.3 Add the archive workflow skill and consumer guide; extend project verification guidance.

## 2. Validation

- [x] 2.1 Run make format, make lint, and make test; validate OpenSpec.

Local MCP tests, formatting, lint, and strict spec validation passed. The local
workspace test build exhausted disk space while linking unrelated API tests;
the required full workspace run is delegated to PR CI before merge.
