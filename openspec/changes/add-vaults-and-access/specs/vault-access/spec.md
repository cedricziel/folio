# Spec Delta

## Purpose

Vaults are the containers that hold Folio's documents, and per-vault roles decide who may see and change what is inside each one. This capability covers vaults, roles and grants, how access is evaluated and reported, moving documents between vaults, and how API clients, MCP tools, workflows and the web UI select a vault.

## ADDED Requirements

### Requirement: Every document belongs to one vault

The system SHALL store each document in exactly one vault. Tags, correspondents, document types, workflows, saved searches and search embeddings SHALL also belong to exactly one vault, and a document SHALL only reference vocabulary entries from its own vault.

#### Scenario: Two vaults with the same tag name

- **WHEN** vault `Home` has a tag `Tax` and vault `Business` has a tag `Tax`
- **THEN** they are two separate tags with different IDs, and listing tags in `Home` returns only the `Home` tag

#### Scenario: Tag from another vault

- **WHEN** an editor of both `Home` and `Business` posts the ID of a `Business` tag to a document in `Home`
- **THEN** the response status is 404 and the document's tags are unchanged

### Requirement: Vault roles

Each grant SHALL carry one of four roles, ordered owner > editor > contributor > viewer. Viewer reads documents and metadata. Contributor also uploads and creates documents, and edits, classifies, reprocesses or deletes only documents it uploaded. Editor reads, writes and deletes all documents and manages vocabularies. Owner also manages members and workflows and renames or deletes the vault.

#### Scenario: Viewer reads but cannot change

- **WHEN** a viewer of `Home` sends `GET /api/vaults/{home}/documents` and then `DELETE /api/documents/{id}` for a document in `Home`
- **THEN** the first response status is 200 and the second is 403, and the document still exists

#### Scenario: Contributor edits its own upload

- **WHEN** a contributor of `Home` uploads a document and then sets its correspondent
- **THEN** both requests succeed

#### Scenario: Contributor edits someone else's upload

- **WHEN** a contributor of `Home` sets the correspondent or deletes a document that another user uploaded to `Home`
- **THEN** the response status is 403 and the document is unchanged

#### Scenario: Contributor and migrated documents

- **WHEN** a contributor of `Archive` tries to edit a document that the upgrade migration moved into `Archive`
- **THEN** the response status is 403, because migrated documents have no uploader

#### Scenario: Editor manages vocabularies

- **WHEN** an editor of `Home` posts `{"name": "Insurance"}` to `/api/vaults/{home}/tags`
- **THEN** the response status is 200 and the tag exists in `Home`

#### Scenario: Contributor cannot create vocabulary entries

- **WHEN** a contributor of `Home` posts `{"name": "Insurance"}` to `/api/vaults/{home}/tags`
- **THEN** the response status is 403

#### Scenario: Editor cannot manage members

- **WHEN** an editor of `Home` sends `PUT /api/vaults/{home}/members/user/{id}` with `{"role": "viewer"}`
- **THEN** the response status is 403 and no grant is created

### Requirement: Grants to users and groups

A vault owner SHALL grant a role to a user or to a group. A principal SHALL hold at most one grant per vault; granting again replaces the role. The user's effective role in a vault SHALL be the highest of their direct grant and the grants of every group they belong to at the time of the request.

#### Scenario: Group grant raises a direct grant

- **WHEN** Alice has a direct viewer grant on `Home` and her group `family` has an editor grant on `Home`
- **THEN** Alice's effective role in `Home` is editor

#### Scenario: Leaving the group

- **WHEN** Alice is removed from `family` and she has only a direct viewer grant on `Home`
- **THEN** her next request to `Home` is evaluated as viewer

#### Scenario: No grant at all

- **WHEN** Bob has no direct grant on `Home` and none of his groups has one
- **THEN** Bob has no role in `Home` and cannot see it

### Requirement: Group sources

The system SHALL know local groups, managed in Folio, and external groups, reported by the identity provider through the OIDC groups claim or the forward-auth `Remote-Groups` header. A local and an external group with the same name SHALL be different groups. External memberships SHALL be replaced with the claim at each OIDC sign-in and with the header on each forward-auth request.

#### Scenario: OIDC claim changes

- **WHEN** Alice signs in through OIDC and her groups claim no longer contains `family`
- **THEN** she loses every role she held only through the external group `family`

#### Scenario: Forward-auth header

- **WHEN** a request from the trusted proxy carries `Remote-Groups: family,finance` for Alice
- **THEN** that request is evaluated with Alice as a member of the external groups `family` and `finance`

#### Scenario: Name collision between sources

- **WHEN** the external group `admins` is reported for Bob and only the local group `admins` holds an owner grant on `Home`
- **THEN** Bob has no role in `Home`

#### Scenario: Grant to an external group nobody has used yet

- **WHEN** an owner sends `PUT /api/vaults/{home}/members/group/external:finance` with `{"role": "viewer"}` before any member of `finance` has signed in
- **THEN** the response status is 200 and the first member of `finance` who signs in can read `Home`

### Requirement: Invisible vaults return 404

When the caller has no role in a vault, every route addressing that vault or anything in it SHALL respond 404 with the same body as for an ID that does not exist. When the caller has a role that does not permit the action, the route SHALL respond 403.

#### Scenario: Vault without a role

- **WHEN** Bob, who has no role in `Home`, requests `GET /api/vaults/{home}`
- **THEN** the response status is 404 and the body equals the body for a random vault ID

#### Scenario: Document in a vault without a role

- **WHEN** Bob requests `GET /api/documents/{id}`, `GET /documents/{id}/thumbnail` or `GET /api/documents/{id}/download` for a document in `Home`
- **THEN** each response status is 404

#### Scenario: Role too low

- **WHEN** a viewer of `Home` sends `PATCH /api/vaults/{home}` with `{"name": "House"}`
- **THEN** the response status is 403 and the vault keeps its name

### Requirement: Instance admins see vault names only

An instance admin without a grant SHALL be able to list every vault's ID, name, owners and creation time at `GET /api/admin/vaults` (status 200) but SHALL NOT read the vault's documents, vocabularies, members list or audit log. Non-admins SHALL get 404 from `/api/admin/vaults`.

#### Scenario: Admin lists vaults

- **WHEN** an instance admin with no grants requests `GET /api/admin/vaults`
- **THEN** the response status is 200 and lists every vault with its name and owners

#### Scenario: Admin opens a vault's documents

- **WHEN** an instance admin with no grant on `Home` requests `GET /api/vaults/{home}/documents`
- **THEN** the response status is 404

#### Scenario: Non-admin calls the admin listing

- **WHEN** a user who is not an instance admin requests `GET /api/admin/vaults`
- **THEN** the response status is 404

### Requirement: Orphaned vault recovery

A vault SHALL always have at least one owner granted directly to a user. When it has none, because the last owner's account was deleted, an instance admin SHALL assign one at `POST /api/admin/vaults/{vault_id}/owner` with `{"user_id": ...}` (200). The call SHALL respond 409 when the vault still has a direct user owner.

#### Scenario: Owner account deleted

- **WHEN** the only owner of `Home` is deleted and an admin posts `{"user_id": <carol>}` to `/api/admin/vaults/{home}/owner`
- **THEN** the response status is 200, Carol is an owner of `Home`, and the audit log records the recovery with the admin as actor

#### Scenario: Recovery on a vault that has an owner

- **WHEN** an admin posts to `/api/admin/vaults/{home}/owner` while Alice is a direct owner of `Home`
- **THEN** the response status is 409 and no grant changes

### Requirement: Vault JSON API

`GET /api/vaults` SHALL list the vaults where the caller has a role, each with `id`, `name` and the caller's `role` (200). `POST /api/vaults` with `{"name"}` SHALL create a vault owned by the caller (201). `GET /api/vaults/{vault_id}` SHALL return one vault (200). `PATCH` with `{"name"}` SHALL rename it (200, owner only).

#### Scenario: Create a vault

- **WHEN** Alice posts `{"name": "Business"}` to `/api/vaults`
- **THEN** the response status is 201 and Alice is the vault's owner

#### Scenario: Empty name

- **WHEN** a client posts `{"name": ""}` to `/api/vaults`
- **THEN** the response status is 400

#### Scenario: List my vaults

- **WHEN** Alice is owner of `Home` and viewer of `Business` and requests `GET /api/vaults`
- **THEN** the response status is 200 and lists `Home` with role `owner` and `Business` with role `viewer`, and no other vault

### Requirement: Deleting a vault

`DELETE /api/vaults/{vault_id}` SHALL delete an empty vault together with its vocabularies, workflows, saved searches and grants (204, owner only). It SHALL respond 409 while the vault holds documents.

#### Scenario: Delete an empty vault

- **WHEN** the owner of an empty vault `Old` sends `DELETE /api/vaults/{old}`
- **THEN** the response status is 204 and `Old` no longer appears in `GET /api/vaults` for any user

#### Scenario: Delete a vault with documents

- **WHEN** the owner of `Home` sends `DELETE /api/vaults/{home}` while `Home` holds a document
- **THEN** the response status is 409 and the vault is unchanged

### Requirement: Membership JSON API

`GET /api/vaults/{vault_id}/members` SHALL list grants with principal type, ID, display name and role for any member (200). Owners SHALL grant or change a role with `PUT /api/vaults/{vault_id}/members/{user|group}/{principal_id}` and `{"role"}` (200) and revoke with `DELETE` on the same path (204).

#### Scenario: Grant a user

- **WHEN** the owner of `Home` sends `PUT /api/vaults/{home}/members/user/{bob}` with `{"role": "contributor"}`
- **THEN** the response status is 200 and Bob's next request to `Home` is evaluated as contributor

#### Scenario: Unknown role

- **WHEN** the owner sends `{"role": "admin"}`
- **THEN** the response status is 400

#### Scenario: Viewer reads the member list

- **WHEN** a viewer of `Home` requests `GET /api/vaults/{home}/members`
- **THEN** the response status is 200

#### Scenario: Revoke takes effect on the next request

- **WHEN** the owner revokes Bob's grant on `Home` while Bob has the document list open
- **THEN** Bob's next request to `Home` responds 404

### Requirement: The last direct owner cannot be removed

The system SHALL reject, with 409, revoking or downgrading the last owner grant held directly by a user. Owner grants held by groups SHALL NOT count, because external group membership can change outside Folio.

#### Scenario: Last owner leaves

- **WHEN** Alice is the only direct owner of `Home`, the group `family` also holds owner, and Alice sends `DELETE /api/vaults/{home}/members/user/{alice}`
- **THEN** the response status is 409 and Alice stays owner

#### Scenario: Second owner exists

- **WHEN** Alice and Bob are both direct owners of `Home` and Alice downgrades herself to editor
- **THEN** the response status is 200 and Bob remains owner

### Requirement: Moving a document between vaults

`POST /api/documents/{id}/move` with `{"vault_id"}` SHALL move the document, its file, thumbnails and embedding to the target vault (200) when the caller is editor or owner in both vaults. It SHALL respond 404 when the caller has no role in either vault and 403 when a role is too low.

#### Scenario: Editor in both vaults

- **WHEN** Alice is editor of `Home` and owner of `Business` and moves a `Home` document to `Business`
- **THEN** the response status is 200, the document is listed in `Business` and not in `Home`, and it is still found by search in `Business`

#### Scenario: Contributor in the source vault

- **WHEN** Bob is contributor of `Home` and editor of `Business` and moves his own upload from `Home` to `Business`
- **THEN** the response status is 403 and the document stays in `Home`

#### Scenario: Target vault not visible

- **WHEN** Alice is editor of `Home` and has no role in `Secret` and moves a `Home` document to `Secret`'s ID
- **THEN** the response status is 404 and the document stays in `Home`

#### Scenario: Move to the same vault

- **WHEN** an editor moves a document to the vault it is already in
- **THEN** the response status is 200 and nothing changes

### Requirement: Document routes outside the vault path

`/api/documents/{id}` and its sub-routes (`download`, `correspondent`, `document_type`, `tags`, `move`) and the HTML routes `/documents/:id/...` SHALL look up the document's vault, apply the caller's role in that vault, and behave like the vault-scoped equivalent.

#### Scenario: Read through the convenience route

- **WHEN** a viewer of `Home` requests `GET /api/documents/{id}` for a document in `Home`
- **THEN** the response status is 200 and the body includes `vault_id`

#### Scenario: Reprocess as a viewer

- **WHEN** a viewer of `Home` posts to `/documents/{id}/reprocess` for a document in `Home`
- **THEN** the response status is 403 and no job is enqueued

### Requirement: Default vault

Each user SHALL have a default vault: a vault where they hold a role. The first admin created by first-run setup SHALL own a new vault `Personal`, which is their default unless they already have a default vault. Every user created later SHALL also get an owned `Personal` vault as default. When the default vault is deleted or the user loses access, the default SHALL become the first vault by name where the user has a role, or none.

#### Scenario: Fresh install

- **WHEN** first-run setup creates the first admin on an instance with no documents
- **THEN** the admin owns a vault named `Personal` and it is their default vault

#### Scenario: Access to the default vault is revoked

- **WHEN** Bob's default vault is `Home`, his grant on `Home` is revoked, and he still has roles in `Business` and `Archive`
- **THEN** Bob's default vault is `Archive`

### Requirement: Upgrade moves existing data into Archive

The upgrade migration SHALL create a vault named `Archive`, put every existing document, tag, correspondent, document type, embedding, workflow and saved search into it, and make the first instance admin its owner and default vault. When no user exists yet, the archive SHALL remain ownerless. First-run setup SHALL provision only the admin's `Personal` vault; it SHALL NOT claim archives by name.

#### Scenario: Upgrade with existing documents and an admin

- **WHEN** an instance with 120 documents and an admin is upgraded
- **THEN** a vault `Archive` holds all 120 documents and their tags, the admin is its owner, and `Archive` is the admin's default vault

#### Scenario: Upgrade before any user exists

- **WHEN** an instance with documents but no users is upgraded and first-run setup then creates the first admin
- **THEN** the admin owns `Personal` as their default vault, and `Archive` remains ownerless

#### Scenario: Upgrade of an empty instance

- **WHEN** an instance without documents or vocabularies is upgraded
- **THEN** no `Archive` vault is created

### Requirement: Audit log

The system SHALL record, with actor user, time, vault and target: vault creation, rename and deletion; every grant, role change and revocation; owner recovery; and every document move with source and target vault. Vault owners SHALL read their vault's entries at `GET /api/vaults/{vault_id}/audit` (200); other roles SHALL get 403.

#### Scenario: Role change is recorded

- **WHEN** Alice changes Bob's role on `Home` from viewer to editor
- **THEN** `GET /api/vaults/{home}/audit` returns an entry naming Alice, Bob, the old role `viewer`, the new role `editor` and the time

#### Scenario: Move is recorded in both vaults

- **WHEN** Alice moves a document from `Home` to `Business`
- **THEN** the audit entries of both `Home` and `Business` show the move with Alice as actor

#### Scenario: Editor reads the audit log

- **WHEN** an editor of `Home` requests `GET /api/vaults/{home}/audit`
- **THEN** the response status is 403

### Requirement: Access is evaluated per request

The system SHALL evaluate the caller's effective role on every request from the current grants and group memberships, for sessions and OAuth access tokens alike. Tokens SHALL identify the user and SHALL NOT carry vault roles.

#### Scenario: Token issued before a revocation

- **WHEN** Bob's MCP client holds a valid access token and his grant on `Home` is revoked
- **THEN** the next request with that token that addresses `Home` responds 404, without the token being reissued

### Requirement: MCP tools select a vault

Every MCP tool that reads or writes documents, vocabularies or search SHALL accept an optional `vault` argument holding a vault ID or name, defaulting to the user's default vault. A `list_vaults` tool SHALL return the vaults where the user has a role, with ID, name, role and which one is the default.

#### Scenario: No vault argument

- **WHEN** Alice, whose default vault is `Home`, calls the search tool without `vault`
- **THEN** only documents in `Home` are searched

#### Scenario: Vault by name

- **WHEN** Alice calls the search tool with `vault: "Business"` and has a role in one vault named `Business`
- **THEN** only documents in `Business` are searched

#### Scenario: Ambiguous name

- **WHEN** Alice has roles in two vaults named `Business` and passes `vault: "Business"`
- **THEN** the tool returns an error that lists both vault IDs and does nothing

#### Scenario: Vault without a role

- **WHEN** Alice passes the name or ID of a vault where she has no role
- **THEN** the tool returns the same "vault not found" error as for a name that does not exist

#### Scenario: No default vault

- **WHEN** a user with no role in any vault calls a document tool without `vault`
- **THEN** the tool returns an error saying no vault is available

#### Scenario: List vaults

- **WHEN** Alice calls `list_vaults`
- **THEN** it returns each vault where she has a role with its ID, name, her role, and a flag on her default vault

### Requirement: Workflows are scoped to a vault

Each workflow SHALL belong to one vault, run only for events on documents in that vault, and be created, changed or deleted only by vault owners. Every webhook payload SHALL include `"vault": {"id", "name"}` of the document's vault.

#### Scenario: Upload triggers only the vault's workflows

- **WHEN** a document is uploaded to `Home` and both `Home` and `Business` have an on-upload workflow
- **THEN** only the `Home` workflow runs

#### Scenario: Webhook payload

- **WHEN** a `Home` workflow sends a webhook for a document
- **THEN** the JSON payload contains `"vault": {"id": "<home id>", "name": "Home"}`

#### Scenario: Editor edits a workflow

- **WHEN** an editor of `Home` tries to change a `Home` workflow
- **THEN** the response status is 403

### Requirement: Saved searches are scoped to a vault

Any member of a vault SHALL be able to save searches in it. Each saved search SHALL belong to one vault and to the user who saved it, be visible only to that user, and run only against that vault. A user who loses every role in the vault SHALL no longer see its saved searches.

#### Scenario: Viewer saves a search

- **WHEN** a viewer of `Home` saves the search `insurance`
- **THEN** the saved search appears for that viewer and for no other member of `Home`

#### Scenario: Saved search after losing access

- **WHEN** Bob saved a search in `Home` and his grant on `Home` is revoked
- **THEN** the saved search no longer appears for Bob and cannot be run

### Requirement: Vault switcher and role badges

Every page SHALL show a sidebar switcher listing the vaults where the user has a role, each with a badge for the user's role, and SHALL mark the current vault. Choosing a vault SHALL open `/vaults/{vault_id}`. `/` SHALL redirect to the user's default vault.

#### Scenario: Switch vault

- **WHEN** Alice picks `Business` in the switcher
- **THEN** the browser opens `/vaults/{business}` and the document list shows only `Business` documents

#### Scenario: Open the home page

- **WHEN** Alice, whose default vault is `Home`, opens `/`
- **THEN** she is redirected to `/vaults/{home}`

#### Scenario: Controls follow the role

- **WHEN** a viewer opens a document page
- **THEN** the page shows a `viewer` badge and no upload, edit, delete or move controls

### Requirement: Members page

`/vaults/{vault_id}/members` SHALL list the vault's grants for any member. For owners it SHALL also offer forms to grant a role to a user or group, change a role, and revoke a grant, each answered with a Turbo Stream that updates the list.

#### Scenario: Owner grants a group

- **WHEN** an owner of `Home` grants `viewer` to the external group `family` on the members page
- **THEN** the list shows `family` with an external-group marker and role `viewer`

#### Scenario: Removing the last owner from the page

- **WHEN** the only direct owner tries to remove themselves on the members page
- **THEN** the page shows an error that a vault needs an owner and the grant stays
