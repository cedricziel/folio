## Dependencies and delivery

Worktree: `/private/tmp/folio-typed-metadata`, branch `codex/typed-metadata`.
Foundation chat: `01a19e17-80c0-7302-85f5-4eff22bc5608`.
Versioning chat: `01a19e18-494a-76d0-9f6c-63df47f79bdd`.

Integrate shared schema and API changes after foundation merges. Deliver storage and domain validation, JSON and web behavior, then native Apple behavior in reviewable PRs. Completion requires all three, their verification, and merges.

## Storage and lifecycle

Definitions, options, and values have explicit vault_id, created_at, updated_at, deleted_at, created_by, and updated_by. Actor IDs come from the authenticated write context; unknown legacy or system actors remain null. Definitions have a UUID, name, kind, required flag, and unique flag. Options have stable UUIDs, labels, and order within their definition.

Use typed columns with a discriminator and database CHECK constraints, not an arbitrary JSON payload. A value belongs to one definition and one logical document. Composite vault-qualified foreign keys prevent cross-vault document, definition, and option references. Only one live value may exist per document and definition. Values cleared by a user retain lifecycle attribution. Permanent document purge removes its values and clears incoming link values transactionally.

Live field names are unique within a vault after trimming and case normalization. Retiring a definition hides it from new editing but retains its values for read-only display. Restore returns a conflict if the name has been reused. A field kind cannot change while any retained value exists, including a cleared value. Create a replacement definition to migrate kinds explicitly. Required or unique constraints cannot be enabled while existing live documents violate them. Required applies to a complete metadata edit, not upload ingestion, so new documents may start incomplete. The UI marks incomplete required fields.

Retired options retain IDs and existing values. They cannot be newly assigned. Rename changes labels without changing assignments. Select options belong to exactly one field. Restoring an option checks live label uniqueness within that field.

## Values and validation

Wire values use a discriminated union with explicit kind and a typed value. Null clears an optional field; absence in a patch leaves it unchanged. Empty text is a value, not null. Required text rejects whitespace-only content. Text is at most 1,024 Unicode scalar values; long text is at most 65,536. Names and option labels are trimmed, nonempty, and at most 128 scalar values.

Integers use signed 64-bit storage and decimal strings on the wire to avoid client number precision loss. Decimal and money amounts use canonical decimal strings with at most 38 significant digits and 18 fractional digits. Reject exponents, NaN, infinity, whitespace, and excess precision. Normalize leading integer zeros, trailing fractional zeros, and negative zero. Never convert amounts through binary floating point. Money also requires a three-letter uppercase currency code. Currency codes are explicit accounting identifiers, validated syntactically; the API does not claim current ISO registry validation or perform exchange conversion.

Dates are real calendar dates in YYYY-MM-DD form with no time zone. Boolean distinguishes false from missing. URL accepts absolute HTTP or HTTPS URLs without credentials. Render URLs as escaped links. Document links accept a logical document UUID in the same authorized vault and never a revision UUID. Self-links are permitted. A trashed target remains a retained link with a Trash indicator, but cannot receive new links. Hidden, missing, and cross-vault targets return the same not-found response without title or vault disclosure.

Unique values compare canonical forms per definition within a vault. Text comparison is case-sensitive; decimal values use normalized strings; money includes currency; links and options compare stable IDs. Missing values do not collide. Retained values on trashed documents reserve uniqueness until cleared or purged, so restoring a document cannot introduce duplicates. Definition/option deletion and constraint changes serialize with value writes. Enforce uniqueness in the database as well as in validation.

## Document lifecycle

Values and links attach to documents.id. Replacing or restoring file revisions does not rewrite metadata. Source document trash retains all values. Target trash retains incoming links and restore reconnects them automatically. Purging a target clears incoming link values with a system actor in the same transaction. A required link can become incomplete after purge, and the UI must show that state. Purging a source cascades its own values.

A vault move must reject a document with retained metadata values or incoming links with 409 until those references are explicitly cleared. This avoids silently carrying definitions into another vault or disclosing target details. If foundation has no move operation, database constraints still reject out-of-vault updates, and future move implementations must adopt this rule. Metadata's move guard is a dependency contract for foundation and versioning integration.

## API and authorization

JSON collections use `/api/vaults/{vault_id}/metadata-fields`; item updates and retirement use `/{field_id}`. Options are managed under `/{field_id}/options`. Restore is an explicit POST to `/restore` on a retired field or option. Document reads and atomic patches use `/api/vaults/{vault_id}/documents/{document_id}/metadata`. Definitions are managed by vault managers; document values use the same per-document edit permission as existing metadata. Readers can display definitions and values. Membership and target access are checked on every request.

GET returns 200, POST creation returns 201, PATCH returns 200, DELETE retirement returns 204, and POST restore returns 200. Invalid values return 400, absent authentication 401, insufficient role in a visible vault 403, missing or inaccessible resources 404, and uniqueness or lifecycle conflicts 409. Atomic patches reject the whole request if any value is invalid. Include a metadata revision or equivalent compare-and-swap token; stale edits return 409.

An explicit metadata search endpoint accepts a definition ID and typed exact value, with bounded cursor pagination. It returns authorized active logical documents. Missing definitions invalidate the query rather than broaden it. Decimal range filters and full-text indexing of custom values are deferred. Existing MCP document responses remain compatible; expose metadata through typed tools only when shared access rules can be reused.

Register request and response schemas in openapi/openapi.json. Apple generation uses the existing schema symlink and generator configuration; verify generated code by building the package. No handwritten alternative wire schema.

## Web and Apple experience

Place Fields alongside existing vault taxonomy management, not global server settings. List name, type, required/unique state, and retirement state. Create and edit forms offer type-specific option controls, inline validation, and explicit retire/restore actions. Do not offer unavailable type changes.

The document inspector has a Custom fields section with text inputs, multiline editors, a three-state boolean control, exact numeric text inputs, date picker, currency plus amount, select menu, safe URL links, and an accessible document picker. Empty and false must look different. Preserve draft input when a save fails and explain edit conflicts. Link navigation stays within the selected vault and opens the logical document's current revision. Trash targets show a clear retained-link state.

Reuse web form components, spacing, typography, and colors from design/tokens.json and corresponding FolioTokens. Apple uses the existing scoped service and workspace navigation. Do not add colors or spacing literals. Viewers see values and links without mutation controls. Test narrow layouts, keyboard navigation, and accessible labels.
