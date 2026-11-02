## 1. Scoped cleanup

- [x] 1.1 Write failing `delete_preserves_unrelated_pre_deleted_mail` and `move_preserves_unrelated_pre_deleted_mail` integration tests, then replace unrestricted expunge with capability-gated UID cleanup.
- [x] 1.2 Add loopback protocol regressions for MOVE, UIDPLUS fallback, unsupported capabilities, and command rejection.
- [x] 1.3 Document capability behavior and the temporary mitigation. Commit the focused fix after checks pass.

## 2. Verification

- [x] 2.1 Run isolated IMAP integration tests with Docker required, `make format`, and focused folio-infra lint.
- [ ] 2.2 Run `make lint` and `make test` after integration.
- [ ] 2.3 Validate the OpenSpec change, archive it as the final commit, and validate the resulting spec.
