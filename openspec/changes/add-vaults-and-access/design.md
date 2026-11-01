# Design

## Context

Today Folio has no notion of ownership. `DocumentRepository`, `TagRepository`, `CorrespondentRepository` and `DocumentTypeRepository` in `crates/folio-core/src/ports/repository.rs` read and write instance-wide tables; vocabulary names are `UNIQUE` across the whole instance (`migrations/001_initial.sql`); `SearchService::search` scores every row of `document_embeddings`; `FileStorage::store` in `crates/folio-infra/src/storage/mod.rs` writes `<document id>/<file>`; `/events` broadcasts to every subscriber.

This change builds on work that is not merged yet:

- **Auth PRs**: `users` with an instance-admin flag, sessions, first-run setup, local groups, OIDC sign-in with a groups claim, forward-auth with a trusted-proxy check and `Remote-Groups`, OAuth tokens for the MCP server. Vaults need a `UserId` on every request; they do not define how it is obtained.
- **PostgreSQL platform PR**: per-backend migration folders `migrations/sqlite/` and `migrations/postgres/` and a backend-agnostic repository wiring.
- **S3 storage adapter** (in progress): stores files under opaque keys recorded on the document.
- **Workflows/webhooks, saved searches, MCP server**: planned or in flight. This change specifies their vault scoping; whichever lands second carries the integration (see tasks group 6).

See proposal.md for motivation.

## Goals / Non-Goals

**Goals:**

- One place in core decides what a role may do, and one place in the HTTP layer turns "no role" into 404.
- Repository code cannot forget the vault: the vault is a required argument, not an optional filter.
- Identical behavior on SQLite and PostgreSQL, proven by one shared contract test.
- Revoking a grant takes effect on the caller's next request.

**Non-Goals:**

- Vault-restricted OAuth tokens (a token limited to some vaults or to read-only). Tokens carry the user only. This is a candidate for a later change; the `AccessPolicy` signature leaves room for it (see Decisions).
- Search across several vaults in one request.
- Per-document permissions or sharing a single document outside its vault.
- Per-vault AI settings, quotas, or encryption keys.
- Managing local groups (create group, add member): that belongs to the auth PRs. This change only reads group membership and grants roles to groups.

## Decisions

### Data model

New tables, in a new migration `NNN_vaults.sql` written twice, once in `migrations/sqlite/` and once in `migrations/postgres/`; `NNN` is the next free number after the auth and PostgreSQL PRs:

- `vaults(id, name, created_by, created_at, updated_at)`: `name` is not unique. A unique name would let any user probe for vault names through 409 responses, and two people may well both want a vault called `Personal`.
- `groups(id, name, source, created_at)` with `source IN ('local','external')` and `UNIQUE(source, name)`. If the auth PR already created `groups`, this migration only adds `source`.
- `group_members(group_id, user_id, origin, synced_at)` with `origin IN ('local','oidc','forward_auth')`. External rows are replaced on each OIDC sign-in and on each forward-auth request; local rows are managed by the auth PR's group UI.
- `vault_members(vault_id, principal_type, user_id, group_id, role, granted_by, granted_at)` with `principal_type IN ('user','group')`, a `CHECK` that exactly one of `user_id`/`group_id` is set, `role IN ('owner','editor','contributor','viewer')`, and partial unique indexes on `(vault_id, user_id)` and `(vault_id, group_id)`. Two nullable foreign keys rather than one polymorphic `principal_id`, so both backends enforce `ON DELETE CASCADE` when a user or group is deleted.
- `audit_events(id, at, actor_user_id, action, vault_id, target_type, target_id, details)`. `details` is JSON (`TEXT` on SQLite, `JSONB` on PostgreSQL). Entries hold IDs and role names, not document titles or content.
- `users.default_vault_id` (nullable, `ON DELETE SET NULL`).

Changes to existing tables:

- `documents` gains `vault_id NOT NULL REFERENCES vaults(id)` and `uploaded_by REFERENCES users(id) ON DELETE SET NULL`, plus an index on `(vault_id, created_at)`.
- `tags`, `correspondents`, `document_types` gain `vault_id NOT NULL` and swap `UNIQUE(name)` for `UNIQUE(vault_id, name)`. SQLite cannot drop a column constraint, so the SQLite migration rebuilds these three tables (create `_new`, copy, drop, rename, recreate indexes) inside the migration transaction with `PRAGMA foreign_keys` handled as sqlx requires; PostgreSQL uses `ALTER TABLE ... DROP CONSTRAINT`.
- `document_embeddings` keeps its shape. Search joins `documents` on `vault_id`; denormalizing `vault_id` into embeddings would save a join but would need updating on every move. Revisit if a vector index (pgvector) needs it.
- `workflows` and `saved_searches`, if they exist when this lands, gain `vault_id NOT NULL`; `saved_searches` also gains `user_id`.

Existing rows: the migration creates the `Archive` vault only when any document or vocabulary row exists, sets `vault_id` on every row to it, and grants `owner` to the oldest user with the admin flag. If there is no user yet, the vault is created with no members and first-run setup grants it (see the vault-access spec). `uploaded_by` stays `NULL` for migrated documents, which is why contributors cannot edit them.

Alternative considered: a `vault_id` column with a default "global" vault and nullable grants, so existing installs need no data migration. Rejected: it keeps a vault nobody explicitly owns and special-cases it in every check.

### Roles and the access port

In `folio-core`:

- `models::vault`: `VaultId`, `Vault`, `VaultRole` (`Ord`: `Viewer < Contributor < Editor < Owner`), `Principal::{User(UserId), Group(GroupId)}`, `VaultMember`, `VaultAction`.
- `VaultAction` is a closed enum: `Read`, `CreateDocument`, `EditDocument { uploaded_by: Option<UserId> }`, `DeleteDocument { uploaded_by }`, `ManageVocabulary`, `SaveSearch`, `ManageWorkflows`, `MoveDocumentOut`, `MoveDocumentIn`, `ReadMembers`, `ManageMembers`, `ReadAudit`, `RenameVault`, `DeleteVault`.
- `VaultRole::permits(&self, action, user) -> bool` is a pure function holding the whole matrix, unit-tested exhaustively (every role × every action).
- `ports::access::AccessPolicy`:

  ```rust
  #[async_trait]
  pub trait AccessPolicy: Send + Sync {
      async fn effective_role(&self, user: &UserId, vault: VaultId) -> Result<Option<VaultRole>>;
      async fn can(&self, user: &UserId, vault: VaultId, action: VaultAction) -> Result<Access>;
  }
  pub enum Access { Allowed(VaultRole), Forbidden(VaultRole), NotVisible }
  ```

  `can` has a default implementation: `effective_role`, then `None → NotVisible`, `Some(r) if r.permits(..) → Allowed(r)`, otherwise `Forbidden(r)`. The three-way result is what lets the API layer return 404 vs 403 without a second query.
- `ports::repository` gains `VaultRepository`, `MembershipRepository` (grants, last-owner count, external group sync) and `AuditLog`.

In `folio-infra`, `SqlAccessPolicy::effective_role` is one query: `MAX(role rank)` over `vault_members` matching the user directly or through `group_members`. No cache: one indexed query per request is cheap next to OCR or embedding calls, and it gives instant revocation. Forward-auth requests carry their groups in the header; the auth adapter upserts them into `group_members` (origin `forward_auth`) before the policy runs, so the policy has a single code path.

Instance admin is not a vault role. `AccessPolicy` never consults the admin flag. Admin endpoints (`/api/admin/vaults`, owner recovery) check the flag in their own handlers and read through `VaultRepository::list_all_summaries`, which returns only id, name, owners and timestamps.

Why admins get no implicit access: a self-hosted instance often has one technically minded admin and several family members or colleagues. A contents-visible admin would make a "private" vault private only by convention. The admin keeps control of the instance (backups, users, settings) and can recover an orphaned vault, which is logged in that vault's audit log, so recovery stays visible to the vault's members. An admin with shell access can still read the database; vaults are an application-level boundary, not encryption (see Risks).

Later: vault-restricted tokens would add an optional `TokenScope` to the request context and intersect it with `effective_role` in the extractor; the port signature does not need to change.

### Repository scoping

Every document- and vocabulary-repository method takes `vault: VaultId` as its first argument: `find_by_id(vault, id)`, `list(vault, filter)`, `create(vault, params)`, and so on. Every query has `WHERE vault_id = ?`; joins (`document_tags`, embeddings) join on both IDs. Assigning a tag or correspondent checks in the same statement that the entry's `vault_id` matches the document's, so a cross-vault ID is "not found".

Two exceptions, both explicit methods:

- `DocumentRepository::locate(id) -> Option<VaultId>` for the `/api/documents/{id}` convenience routes and the background worker. It returns only the vault ID; the caller must then run `AccessPolicy::can` before calling a scoped method.
- `DocumentRepository::move_to(id, from, to, mapping)` runs in one transaction: update `vault_id`, remap tags/correspondent/type by name (creating missing entries in the target), write the audit entry.

Alternative considered: keep the current signatures and add a `vault_id` to `DocumentFilter`. Rejected: filters are optional by nature, and a forgotten filter is a data leak that compiles.

The isolation contract test lives in `folio-infra/tests/vault_isolation.rs` as a generic suite instantiated once per backend (SQLite in-memory; PostgreSQL when `DATABASE_URL_POSTGRES` is set, as in CI). It creates vaults A and B with same-named tags, correspondents and types and documents with embeddings in each, then asserts that every repository method called with A never returns, counts, updates or deletes a row of B, including search and `locate`-then-scoped lookups with mismatched IDs.

### HTTP layer

A `VaultAccess` extractor in `folio-api` resolves `{vault_id}` from the path, or the document's vault via `locate` for `/api/documents/{id}` and `/documents/:id`, calls `can`, and maps `NotVisible → 404` with the same body as a missing ID, `Forbidden → 403`. Handlers receive `VaultContext { vault_id, role, user }` and pass `vault_id` to repositories. The 404 body is produced by the same function as for a nonexistent ID so they cannot drift; a test compares the two byte-for-byte.

Routes (JSON unless noted):

| Route | Notes |
|---|---|
| `GET/POST /api/vaults`, `GET/PATCH/DELETE /api/vaults/{vault_id}` | |
| `GET /api/vaults/{vault_id}/members`, `PUT/DELETE .../members/{user\|group}/{principal_id}` | group IDs accept `local:<name>` / `external:<name>`; an unknown external name creates the group row |
| `GET /api/vaults/{vault_id}/audit` | owner only, newest first, paginated with `?before=` |
| `/api/vaults/{vault_id}/documents[/upload\|/search]`, `.../tags`, `.../correspondents`, `.../document_types` | replace the unscoped collection routes |
| `/api/documents/{id}[/download\|/correspondent\|/document_type\|/tags\|/move]` | convenience; vault resolved by `locate` |
| `PUT /api/me/default-vault` | |
| `GET /api/admin/vaults`, `POST /api/admin/vaults/{vault_id}/owner` | 404 for non-admins |
| HTML: `/` (redirect), `/vaults/{vault_id}`, `/vaults/{vault_id}/documents/{upload,search}`, `/vaults/{vault_id}/manage/...`, `/vaults/{vault_id}/members` | Turbo |
| HTML: `/documents/:id/...` | unchanged paths, access enforced |

OpenAPI: `openapi/openapi.json` gains a `Vault`, `VaultRole`, `VaultMember`, `Principal` and `AuditEvent` schema, `vault_id` on `Document`, `Tag`, `Correspondent` and `DocumentType`, the routes above, and drops the unscoped collection paths. Every vault-scoped operation documents 403 and 404. The Swift client is regenerated; the app adds a vault picker that stores the chosen vault and passes it to every collection call, and shows the role to hide actions the role cannot perform.

### MCP and OAuth

The MCP server resolves the token to a `UserId` and builds the same `VaultContext` per tool call. Every document tool's input schema gains `vault: string` (optional). Resolution: parse as UUID and look it up; otherwise exact, case-sensitive name match among vaults where the user has a role; zero matches → the "vault not found" error used for invisible IDs; several → error listing the IDs. `list_vaults` returns `{id, name, role, is_default}`.

### Storage

Keys become `vaults/{vault_id}/documents/{document_id}/{filename}` and `.../thumbnails/{n}.jpg`. The key recorded on the document stays authoritative, so documents stored before the upgrade keep working with their old key. A `relocate_storage` background job (enqueued once by the upgrade, idempotent, resumable) moves old files to the new layout and updates the recorded keys; it can lag without breaking reads.

The storage port gains `rename_prefix(from, to)`: filesystem `rename`, S3 server-side copy then delete. A move first copies to the target prefix, then commits the database transaction, then deletes the old prefix. A crash between commit and delete leaves an orphaned copy at the old prefix; the relocation job also sweeps prefixes with no matching document.

Alternative considered: keep keys free of the vault and never move files. Simpler moves, but a vault could not be backed up, exported or purged by prefix, and the S3 bucket policy could not separate vaults later.

### Events

`DocumentEvent` gains `vault_id`; the worker sets it from `locate`. The SSE handler keeps the subscriber's `UserId` and memoizes its role per vault for the life of the connection. The membership service publishes an in-process `membership_changed { vault_id }` signal on every grant change and on every external group sync that changes a membership; SSE handlers drop their memo for that vault when they see it. This keeps delivery checks free of database queries while still applying a revocation before the next event. `removed`/`added` are emitted by the move service.

### Workflows and webhooks

Workflows get `vault_id`; the trigger matcher filters by the document's vault. Only owners manage workflows because a workflow keeps running and sending documents to external URLs after the person who created it loses access; restricting it to owners keeps that power with the people who control membership. Webhook bodies gain `"vault": {"id", "name"}`.

### UI

The sidebar switcher is an Askama partial fed by `GET /api/vaults`-equivalent data in the page context; role badges use the design-system tokens for status colors (no new literals). The members page uses Turbo Stream forms like `/manage`. The document page adds "Move to vault", listing only vaults where the user is editor or owner.

## Risks / Trade-offs

- [A repository method added later skips the vault filter] → The vault is a required argument, so a method without it stands out in review; the isolation contract test has a case for every public method, and a new method without one is a review finding.
- [404-vs-403 leaks through timing or other endpoints] → Same body; both paths do one lookup. Timing differences are accepted as out of scope for a self-hosted system.
- [Admin assumes they can read everything and is surprised] → Admin vault listing shows "no access" per vault; the docs state the model; owner recovery exists for lockouts.
- [External group names collide with local ones] → Separate namespaces by `source`.
- [Forward-auth header spoofing] → Relies on the auth PR's trusted-proxy check; `Remote-Groups` is ignored unless the request comes from a trusted proxy.
- [SQLite table rebuild in the migration on large instances] → Runs once in a transaction; vocabulary tables are small. Documents only get `ADD COLUMN` plus an `UPDATE`.
- [Breaking JSON routes] → No published clients exist yet (the OpenAPI spec and Swift client are still in review), so no compatibility shim; the change is called out as BREAKING.
- [Vaults are not encryption] → Stated in the docs; anyone with database or bucket access reads everything.

## Migration Plan

1. Land the auth PRs and the PostgreSQL PR.
2. Ship the migration, ports and scoped repositories (tasks groups 1–2) together, so no release has vault columns without scoped queries.
3. On upgrade: the SQL migration creates `Archive` and assigns rows; first-run setup or the oldest admin becomes its owner; the `relocate_storage` job moves files in the background.
4. Rollback: restore the pre-upgrade backup of the database and of the storage directory or bucket together. There is no automated down migration, because undoing the vocabulary split would have to merge same-named entries from different vaults, and `relocate_storage` changes file keys that the old database would no longer find. The release notes ask admins to take both backups before upgrading.

## Open Questions

- Audit retention: keep forever, or prune after N days? Does not affect the schema.
- Should the vault list for instance admins include document counts for capacity planning? Easy to add later as a field.
