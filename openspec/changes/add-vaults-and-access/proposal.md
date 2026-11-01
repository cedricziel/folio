# Proposal

## Why

Folio keeps every document in one shared pool, so anyone with access sees everything. A household or small team needs separate collections (personal papers, the family archive, a business) with different people allowed to read or change each one. With users, groups and OIDC/forward-auth sign-in landing in the auth PRs, vaults are the next step: they turn "who is signed in" into "what may they see and change".

## What Changes

- Add **vaults**: named containers for documents. Every document belongs to exactly one vault and can be moved to another vault by a user who is editor or owner in both.
- Scope tags, correspondents, document types, workflows, saved searches and search embeddings to a vault. Vocabulary names become unique per vault instead of instance-wide.
- Add four per-vault roles: **owner**, **editor**, **contributor**, **viewer**. Grants go to a user or to a group (local groups, OIDC group claims, forward-auth `Remote-Groups`). A user's effective role is the highest of their direct and group grants.
- Instance admins can list vault names and owners but cannot read vault contents unless they hold a grant. They can assign a new owner to a vault that has none left (audited).
- First-run setup creates a personal vault for the first admin. On upgrade, a migration puts all existing documents and vocabularies into a vault named `Archive` owned by the first admin.
- **BREAKING** JSON API: collection routes move under the vault: `/api/vaults/{vault_id}/documents`, `.../documents/upload`, `.../documents/search`, `.../tags`, `.../correspondents`, `.../document_types`. `/api/documents`, `/api/documents/upload`, `/api/documents/search`, `/api/tags`, `/api/correspondents` and `/api/document_types` are removed. `/api/documents/{id}` and its sub-routes stay as a convenience that resolves the vault and enforces access.
- New JSON API routes: `/api/vaults` (list, create, read, rename, delete), `/api/vaults/{vault_id}/members` (list, grant, change, revoke), `/api/vaults/{vault_id}/audit`, `POST /api/documents/{id}/move`, `PUT /api/me/default-vault`, and `/api/admin/vaults` for instance admins.
- A caller with no role in a vault gets **404**, never 403, for the vault and for anything in it, so vault and document IDs do not leak. A caller with a role that is too low gets 403.
- **BREAKING** HTML routes: the document list, upload, search and `/manage` move under `/vaults/{vault_id}/...`; `/` redirects to the user's default vault. `/documents/:id` pages keep their paths and enforce access.
- Web UI: vault switcher in the sidebar with role badges, a members page per vault, a "Move to vault" action on the document page.
- Storage keys gain a vault prefix: `vaults/{vault_id}/documents/{document_id}/...`. Moving a document rewrites its keys.
- SSE events carry `vault_id` and are delivered only to subscribers who can read that vault.
- OAuth access tokens and MCP: tokens carry the user identity and access is evaluated on every request. MCP tools take an optional `vault` argument (name or ID) that defaults to the user's default vault; a new `list_vaults` tool lists the vaults the user can see.
- Webhook payloads sent by workflows include `vault: {id, name}`.
- Audit log of vault creation, renames, deletion, membership changes and document moves (actor, time, target).

## Capabilities

### New Capabilities

- `vault-access`: vaults, roles and grants, effective-role evaluation, the uniform-404 rule, instance-admin visibility, vault and membership JSON API, moving documents, default vaults, the upgrade migration, audit log, vault scoping of workflows, saved searches, OAuth tokens and MCP tools, and the vault UI (switcher, members page, role badges).

### Modified Capabilities

- `document-upload`: upload, creation and listing happen inside a vault and need contributor or higher; storage keys gain the vault prefix; deletion follows the role rules; JSON routes move under `/api/vaults/{vault_id}`.
- `document-metadata`: vocabularies are per vault (names unique per vault); JSON and `/manage` routes move under the vault; only editors and owners manage vocabularies; contributors classify only their own uploads; classification is carried over when a document moves.
- `document-search`: search covers one vault only; JSON and HTML search routes move under the vault.
- `realtime-events`: events carry `vault_id`, are filtered by the subscriber's access, and moves produce `removed`/`added` events.
- `settings`: each user picks a default vault on `/settings`; AI settings stay instance-wide.

## Impact

- **folio-core**: new models `Vault`, `VaultRole`, `Principal` (user or group), `VaultMember`, `Group`, `AuditEvent`, `VaultAction`; new ports `VaultRepository`, `MembershipRepository`, `AuditLog` and `AccessPolicy` (`can(user, vault, action)`); every existing repository method takes a `VaultId`; `Document` gains `vault_id` and `uploaded_by`.
- **folio-infra**: SQLite and PostgreSQL adapters for the new ports and vault-scoped queries in every existing repository; storage adapters (filesystem and the S3 adapter in progress) write `vaults/{vault_id}/...` keys and support moving a document's keys; a shared repository contract test asserts cross-vault isolation on both backends.
- **folio-api**: JSON routes (vaults, members, audit, admin, move, vault-scoped collections) and HTML (Turbo) routes (`/vaults/{vault_id}/...`, members page, switcher); an extractor that resolves the vault, evaluates `AccessPolicy` and maps "no role" to 404; SSE filtering; MCP tool arguments and `list_vaults`; `openapi/openapi.json` and the generated Swift client.
- **Database**: new migration `<next>_vaults.sql` in both `migrations/sqlite/` and `migrations/postgres/` creates `vaults`, `vault_members`, `groups`, `group_members` (if the auth PR has not), `audit_events`; adds `vault_id` to documents, tags, correspondents and document types (and to workflows and saved searches if they exist), adds `documents.uploaded_by`, and replaces the instance-wide name uniqueness with `(vault_id, name)`. Existing rows move into the `Archive` vault.
- **Settings**: the per-user default vault is stored on the user record, not in `config.toml` or the `settings` table. No new instance setting.
- **Depends on**: the auth PRs (users, local groups, OIDC sign-in with group claims, forward-auth with `Remote-Groups`, OAuth tokens for MCP) and the PostgreSQL platform PR (per-backend migration folders). See tasks.md for the order.
- **Clients**: the Swift app and the PWA need a vault picker; the MCP tool schema changes (new optional argument, new tool).
