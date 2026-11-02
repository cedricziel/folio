# Vault selection

A vault contains documents and their tags, correspondents, and document types. The library selector lists only vaults you can access and shows your effective role. A single vault appears as a label.

On the web, the selected vault is part of the URL: `/vaults/{vault_id}/`. On Apple platforms, each workspace keeps its own selection. Switching clears the current document and search. It does not change your account's default vault.

Upload and share sheets have a separate destination. An upload keeps the destination chosen when it starts, including background uploads from the Apple share extension. Choosing another destination does not switch the library.

## Files and Finder

The Folio location lists accessible vaults as top-level folders. Each vault has Inbox, All documents, Tags, Correspondents, and Types folders. These folders are independent of the app's selected vault.

Copy files into a writable vault's Inbox to upload them there. Viewer inboxes are read-only. Rename, move, and delete operations remain unsupported by the provider.

Provider identifiers include the vault UUID, so renaming a vault or switching the app does not redirect an open file. After upgrading from the unscoped tree, reopen files through their vault folders; old unscoped identifiers are invalidated rather than mapped to another vault.

## Roles

| Role | Permissions |
| --- | --- |
| Viewer | Read documents and vocabulary |
| Contributor | Upload documents; edit and delete their own uploads through the API and web UI |
| Editor | Edit and delete any document; manage vocabulary |
| Owner | Editor permissions and workflow/source management |

The Apple app currently offers edit and delete controls to editors and owners. Contributors can upload. The server checks permissions on every request, including after membership changes.

Vault creation, membership administration, and document moves are not yet available in the clients. Noninteractive Shortcuts continue to use the default vault. The Scan shortcut opens the app, where the scan review offers a destination picker. Spotlight entries created by the app retain the document's vault.

## JSON API

`GET /api/vaults` returns `vaults`, an array of `{id, name, role}`, and `default_vault_id`.

Existing document and vocabulary endpoints accept an optional `X-Folio-Vault` UUID header. Without it, requests use the account's default vault. A malformed selection returns 400, an inaccessible or unknown vault returns 404, and an action disallowed by the caller's role returns 403. Selection never changes the account default.

The generated contract is in [openapi/openapi.json](../openapi/openapi.json), also served at `/api/openapi.json`. The browser API reference is at `/api/docs`.

When authentication is disabled, discovery exposes only the instance default vault. Older Apple clients continue to work through default-vault API requests; updated Apple clients also support older servers that do not expose vault discovery.
