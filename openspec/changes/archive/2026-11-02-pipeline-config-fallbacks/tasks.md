## 1. Startup configuration

- [x] 1.1 Write the failing `pipeline_environment_overrides_file_and_keeps_missing_defaults` test, then add typed pipeline sections and registry entries.
- [x] 1.2 Write the failing `saved_rows_override_config_without_resetting_missing_keys` test, then merge database rows over configured pipeline values and render them on the HTML settings page.

## 2. Fresh databases

- [x] 2.1 Write the failing `fresh_database_uses_config_and_preserves_saved_overrides` contract for SQLite and PostgreSQL, then clear only fresh migration defaults while holding migration locks.

## 3. Verification

- [x] 3.1 Document the keys and precedence; run isolated API smoke and the settings screen tour.
- [x] 3.2 Run `make format`, `make lint`, `make test`, and changed-spec validation; commit the implementation and archive the change as the final commit.
