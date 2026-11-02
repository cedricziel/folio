## 1. Core behavior

- [x] 1.1 Add failing group matching, nested pattern, depth, and size regressions; implement bounded recursive conditions.
- [x] 1.2 Verify historical snapshot matching, legacy JSON roundtrip, and recursive OpenAPI references.

## 2. API compatibility

- [x] 2.1 Reproduce group loss with condition_groups_survive_api_storage_and_html_edits; preserve stored groups during HTML edits.
- [x] 2.2 Regenerate OpenAPI and verify the generated Swift client.

## 3. Verification

- [x] 3.1 Run make format, make lint, make test, and relevant isolated project verify suites.
- [x] 3.2 Validate and archive this change.
