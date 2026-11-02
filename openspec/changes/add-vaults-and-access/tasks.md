# Tasks

Each numbered group is one pull request, merged in order. Groups 1 and 2 ship in the same release (no release may have vault columns without scoped queries).

Prerequisites, not part of this change:

- Auth PRs merged: `users` with an admin flag, sessions, first-run setup, local groups, OIDC with a groups claim, forward-auth with a trusted-proxy check and `Remote-Groups`, OAuth tokens. Groups 1, 3 and 5 need them.
- PostgreSQL platform PR merged: `migrations/sqlite/` and `migrations/postgres/` and a Postgres test database in CI. Groups 1 and 2 need it.

Groups 1 and 2 form the foundation release. Existing HTTP and MCP calls use the authenticated user's default vault, provisioned on the first authenticated request. Explicit vault routes, membership endpoints, action-level role enforcement, the vault switcher, MCP vault selection, and new storage prefixes remain in groups 3–6.

## 1. Data model, migration and access port (PR 1)

- [x] 1.1 Add `VaultId`, `Vault`, `VaultRole`, `Principal`, `VaultMember`, `VaultAction` and `Access` to `folio-core/src/models/vault.rs`; first write the failing unit test `role_matrix` that checks every role against every action, including contributor with `uploaded_by` equal to, different from, and `None`, then implement `VaultRole::permits`.
- [x] 1.2 Add the `AccessPolicy` port with the default `can` in `folio-core/src/ports/access.rs`; failing test first: `can_maps_roles_to_access` with a stub `effective_role` returning `None`, `Viewer` and `Owner`.
- [x] 1.3 Add the ports `VaultRepository`, `MembershipRepository` and `AuditLog` to `folio-core/src/ports/repository.rs` (signatures only; compiles with `cargo check -p folio-core`).
- [x] 1.4 Write `NNN_vaults.sql` for SQLite: new tables, `vault_id`/`uploaded_by` on documents, vocabulary table rebuild with `UNIQUE(vault_id, name)`, `users.default_vault_id`, and the `Archive` backfill. Failing test first: `migration_moves_existing_rows_into_archive` loads a pre-change fixture database and asserts every row has the `Archive` vault and the oldest admin owns it; plus `migration_skips_archive_on_empty_db`.
- [x] 1.5 Write the PostgreSQL twin of `NNN_vaults.sql`; the same two migration tests run against Postgres in CI and pass.
- [x] 1.6 Implement `SqlVaultRepository`, `SqlMembershipRepository` (including the last-direct-owner count and external group sync) and `SqlAuditLog` for both backends; failing tests first: `effective_role_is_highest_of_direct_and_group`, `local_and_external_groups_with_same_name_are_distinct`, `last_direct_owner_cannot_be_removed`, `group_owner_does_not_count_as_last_owner`.
- [x] 1.7 Implement `SqlAccessPolicy` (one query, no cache); failing test first: `revoked_grant_is_not_visible_on_next_call`.
- [x] 1.8 Hook first-run setup: create `Personal` for the first admin, grant `Archive` if it has no owner, set the default vault; also create `Personal` for each user created afterwards. Failing tests first: `first_run_creates_personal_vault`, `first_run_claims_ownerless_archive`, `new_user_gets_personal_vault`.
- [x] 1.9 Run `make format`, `make lint`, `make test` and commit as `feat(core): add vaults, roles and the access policy port` (plus `feat(infra): ...` commits per adapter).

## 2. Repository scoping and isolation tests (PR 2, depends on 1)

- [x] 2.1 Write the failing contract suite `folio-infra/tests/vault_isolation.rs`, generic over the backend: two vaults with same-named tags, correspondents, types and documents with embeddings; assert that no scoped method called with vault A returns, counts, updates or deletes a vault B row, and that assigning a vault B tag to a vault A document is "not found".
- [x] 2.2 Change `DocumentRepository`, `TagRepository`, `CorrespondentRepository` and `DocumentTypeRepository` to take `VaultId` on every method, add `DocumentRepository::locate`, and update the SQLite adapters until 2.1 passes on SQLite.
- [x] 2.3 Update the PostgreSQL adapters until 2.1 passes on PostgreSQL in CI.
- [x] 2.4 Scope `SearchService::search` to a vault (join on `documents.vault_id`); add the case `search_never_returns_other_vault` to 2.1 before the change.
- [x] 2.5 Implement `DocumentRepository::move_to` with name-based vocabulary remapping and the audit entry in one transaction; failing tests first: `move_remaps_tags_by_name`, `move_creates_missing_correspondent_in_target`, `move_writes_audit_entry`.
- [x] 2.6 Set `vault_id` and `uploaded_by` on documents created by the worker and the existing upload code paths; the worker uses `locate` before scoped calls. Existing worker tests pass with the new signatures.
- [x] 2.7 Run `make format`, `make lint`, `make test`; commit as `refactor(core): scope repositories by vault` and `test(infra): assert cross-vault isolation`.

## 3. JSON API (PR 3, depends on 2 and the auth PRs)

- [ ] 3.1 Add the `VaultAccess` extractor mapping `NotVisible` to 404 and `Forbidden` to 403; failing test first: `invisible_vault_404_body_equals_unknown_id_body`.
- [ ] 3.2 Add `GET/POST /api/vaults` and `GET/PATCH/DELETE /api/vaults/{vault_id}`; failing tests first for each scenario of the vault-access requirements "Vault JSON API" and "Deleting a vault" (201 create, 400 empty name, 409 non-empty delete, 403 viewer rename, 404 no role).
- [ ] 3.3 Add `/api/vaults/{vault_id}/members` routes; failing tests first for "Membership JSON API" and "The last direct owner cannot be removed" (200 grant, 400 unknown role, 409 last owner, 404 next request after revoke).
- [ ] 3.4 Move document, upload, search and vocabulary collection routes under `/api/vaults/{vault_id}` and remove the unscoped ones; port the existing API tests to the new paths first and add the role scenarios from the document-upload, document-metadata and document-search deltas (viewer 403 on upload, 404 on former paths).
- [ ] 3.5 Enforce access on `/api/documents/{id}` and its sub-routes via `locate`; failing tests first: viewer 200 with `vault_id` in the body, no-role 404 on get, download and thumbnail.
- [ ] 3.6 Add `POST /api/documents/{id}/move`; failing tests first for each "Moving a document between vaults" scenario.
- [ ] 3.7 Add `GET /api/vaults/{vault_id}/audit`, `PUT /api/me/default-vault`, `GET /api/admin/vaults` and `POST /api/admin/vaults/{vault_id}/owner`; failing tests first for the audit, default vault, admin listing and orphan recovery scenarios.
- [ ] 3.8 Filter `/events` by vault access and emit `removed`/`added` on moves; failing tests first: `subscriber_without_role_gets_no_event`, `revoked_subscriber_gets_no_further_events`, `move_sends_removed_and_added`.
- [ ] 3.9 Update `openapi/openapi.json` (schemas, routes, 403/404 responses, removed paths) and verify it matches the spec generated by utoipa (the drift check added by the OpenAPI PR).
- [ ] 3.10 Run `make format`, `make lint`, `make test`; commit as `feat(api)!: scope the JSON API by vault` with a `BREAKING CHANGE:` footer.

## 4. Web UI (PR 4, depends on 3)

- [ ] 4.1 Move the HTML list, upload, search and `/manage` routes under `/vaults/{vault_id}` and redirect `/` to the default vault; failing route tests first for "Open the home page" and the 404/403 cases.
- [ ] 4.2 Add the sidebar vault switcher with role badges, styled with design-system tokens; failing template test first: the switcher lists only vaults with a role and marks the current one.
- [ ] 4.3 Hide upload, edit, delete and move controls the role cannot use; failing test first: a viewer's document page renders none of them.
- [ ] 4.4 Add `/vaults/{vault_id}/members` with Turbo Stream forms; failing tests first: owner grants an external group, last-owner removal shows the error, viewer sees a read-only list.
- [ ] 4.5 Add "Move to vault" on the document page listing only editor/owner vaults; failing test first for the list contents.
- [ ] 4.6 Add the default vault picker to `/settings`; failing test first: saving `Business` makes `/` redirect there.
- [ ] 4.7 Run `make format`, `make lint`, `make test`; commit as `feat(ui): add vault switcher, members page and role badges`.

## 5. MCP and OAuth (PR 5, depends on 3 and the OAuth/MCP PR)

- [ ] 5.1 Add the optional `vault` argument to every document, vocabulary and search tool with ID-then-name resolution; failing tests first for each "MCP tools select a vault" scenario (default, by name, ambiguous, invisible, no default).
- [ ] 5.2 Add the `list_vaults` tool; failing test first for its output shape.
- [ ] 5.3 Add a test that a token issued before a revocation gets "vault not found" on the next call.
- [ ] 5.4 Document the `vault` argument in the MCP tool descriptions and the README; run `make format`, `make lint`, `make test`; commit as `feat(mcp): select a vault per tool call`.

## 6. Workflows, saved searches, storage keys and clients (PR 6, depends on 2; workflow and saved-search parts depend on those features)

- [ ] 6.1 Add `rename_prefix` to the storage port and implement it for the filesystem and S3 adapters; failing adapter tests first, S3 against the existing test harness.
- [ ] 6.2 Write new files under `vaults/{vault_id}/documents/{id}/` and move them in `move_to` (copy, commit, delete); failing tests first: the document-upload scenarios "Filename with spaces and directories" and "File of a moved document".
- [ ] 6.3 Add the idempotent `relocate_storage` job, enqueued once by the upgrade; failing tests first: old keys are moved and recorded, a second run changes nothing, "File stored before the upgrade" still downloads before the job runs.
- [ ] 6.4 Scope workflows by vault, restrict management to owners, add `vault {id, name}` to webhook payloads; failing tests first for each "Workflows are scoped to a vault" scenario. If workflows have not landed, move this task into the workflow change instead.
- [ ] 6.5 Scope saved searches to vault and user; failing test first for "Saved search after losing access". Same rule as 6.4 if saved searches have not landed.
- [ ] 6.6 Regenerate the Swift client and add the vault picker to the Apple app; the app's unit tests for the picker pass.
- [ ] 6.7 Run `make format`, `make lint`, `make test`; commit per part (`feat(infra): prefix storage keys with the vault`, `feat(workflows): scope workflows by vault`, ...).

## 7. Integration check

- [ ] 7.1 On a copy of a pre-upgrade SQLite database and a pre-upgrade PostgreSQL database, start the new release, complete first-run setup, and verify: all documents are in `Archive`, search in `Archive` returns them, a second user without grants sees 404 for `Archive`, and `relocate_storage` finishes.
- [ ] 7.2 Run `make format`, `make lint` and `make test` on the merged result.

## Workflow follow-up

- Archive the change as the last commit of the final PR, so the merged specs and the code land together.
