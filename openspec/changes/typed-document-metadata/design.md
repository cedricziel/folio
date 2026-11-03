## Dependencies and delivery

Worktree: `/private/tmp/folio-typed-metadata`. Validation PR #259 is merged. Field-definition PR #267 uses `codex/metadata-definitions`; transactional field storage uses `codex/metadata-field-storage`.
Foundation chat: `01a19e17-80c0-7302-85f5-4eff22bc5608`.
Versioning chat: `01a19e18-494a-76d0-9f6c-63df47f79bdd`.

Actor #258, ownership #260, and transactional envelope #265 are merged and integrated. Shared schema and API implementation is unblocked. Deliver storage and domain validation, JSON and web behavior, then native Apple behavior in reviewable PRs. Completion requires all three, their verification, and merges.

## Storage and lifecycle

Definitions, options, and values have explicit vault_id, created_at, updated_at, deleted_at, created_by, and updated_by. Actor IDs come from the authenticated write context; unknown legacy or system actors remain null. Definitions have a UUID, name, kind, required flag, and unique flag. Options have stable UUIDs, labels, and order within their definition.

Use typed columns with a discriminator and database CHECK constraints, not an arbitrary JSON payload. A value belongs to one definition and one logical document. Composite vault-qualified foreign keys prevent cross-vault document, definition, and option references. Only one live value may exist per document and definition. Values cleared by a user retain lifecycle attribution. Permanent document purge removes its values and clears incoming link values transactionally.

Live field names are unique within a vault after trimming and case normalization. Retiring a definition prevents new assignments and edits but retains existing values for display. Editors may explicitly clear retained assignments, including assignments to retired fields. Restore returns a conflict if the name has been reused. A field kind cannot change while any retained value exists, including a cleared value. Create a replacement definition to migrate kinds explicitly. Enabling required marks missing values incomplete and applies validation to subsequent complete metadata edits. Enabling unique is rejected when retained values collide. Required applies to a complete metadata edit, not upload ingestion, so new documents may start incomplete. The UI marks incomplete required fields.

Retired options retain IDs and existing values. They cannot be newly assigned. Rename changes labels without changing assignments. Select options belong to exactly one field. Restoring an option checks live label uniqueness within that field.

## Values and validation

Wire values use a discriminated union with explicit kind and a typed value. Null clears an optional field; absence in a patch leaves it unchanged. Empty text is a value, not null. Required text rejects whitespace-only content. Text is at most 1,024 Unicode scalar values; long text is at most 65,536. Names and option labels are trimmed, nonempty, and at most 128 scalar values.

Integers use signed 64-bit storage and decimal strings on the wire to avoid client number precision loss. Decimal and money amounts use canonical decimal strings with at most 38 significant digits and 18 fractional digits. Reject exponents, NaN, infinity, whitespace, and excess precision. Normalize leading integer zeros, trailing fractional zeros, and negative zero. Never convert amounts through binary floating point. Money also requires a three-letter uppercase currency code. Currency codes are explicit accounting identifiers, validated syntactically; the API does not claim current ISO registry validation or perform exchange conversion.

Dates are real calendar dates in YYYY-MM-DD form with no time zone. Boolean distinguishes false from missing. URL accepts absolute HTTP or HTTPS URLs without credentials. Render URLs as escaped links. Document links accept a logical document UUID in the same authorized vault and never a revision UUID. Self-links are permitted. A trashed target remains a retained link with a Trash indicator, but cannot receive new links. Hidden, missing, and cross-vault targets return the same not-found response without title or vault disclosure.

Unique values compare canonical forms per definition within a vault. Text comparison is case-sensitive; decimal values use normalized strings; money includes currency; links and options compare stable IDs. Missing values do not collide. Retained values on trashed documents reserve uniqueness until cleared or purged, so restoring a document cannot introduce duplicates. Definition/option deletion and constraint changes serialize with value writes. Enforce uniqueness in the database as well as in validation.

## Document lifecycle

Values and links attach to documents.id. Replacing or restoring file revisions does not rewrite metadata. Source document trash retains all values. Target trash retains incoming links and restore reconnects them automatically. Purging a target clears incoming link values with a system actor in the same transaction. A required link can become incomplete after purge, and the UI must show that state. Purging a source cascades its own values.

A vault move must reject a document with non-cleared metadata values or incoming links with 409 until those references are explicitly cleared. Clearing a link removes its target reference. Once no values or incoming links remain, the move transaction removes cleared value tombstones and the old vault metadata revision record before changing document ownership. Immutable transition events retain the prior history. This avoids silently carrying definitions into another vault or disclosing target details. The existing DocumentRepository::move_to implementations must check this condition inside their transactions before remapping classifications. Composite foreign keys also reject out-of-vault updates. Metadata's move guard is a dependency contract for foundation and versioning integration.

## API and authorization

JSON collections use `/api/vaults/{vault_id}/metadata-fields`; item updates and retirement use `/{field_id}`. Options are managed under `/{field_id}/options`. Restore is an explicit POST to `/restore` on a retired field or option. Document reads and atomic patches use `/api/vaults/{vault_id}/documents/{document_id}/metadata`. Definitions are managed by vault managers; document values use the same per-document edit permission as existing metadata. Readers can display definitions and values. Membership and target access are checked on every request.

GET returns 200, POST creation returns 201, PATCH returns 200, DELETE retirement returns 204, and POST restore returns 200. Invalid values return 400, absent authentication 401, insufficient role in a visible vault 403, missing or inaccessible resources 404, and uniqueness or lifecycle conflicts 409. Atomic patches reject the whole request if any value is invalid. Include a metadata revision or equivalent compare-and-swap token; stale edits return 409.

An explicit metadata search endpoint accepts a definition ID and typed exact value, with bounded cursor pagination. It returns authorized active logical documents. Missing definitions invalidate the query rather than broaden it. Decimal range filters and full-text indexing of custom values are deferred. Existing MCP document responses remain compatible; expose metadata through typed tools only when shared access rules can be reused.

Register request and response schemas in openapi/openapi.json. Apple generation uses the existing schema symlink and generator configuration; verify generated code by building the package. No handwritten alternative wire schema.

## Web and Apple experience

Place Fields alongside existing vault taxonomy management, not global server settings. List name, type, required/unique state, and retirement state. Create and edit forms offer type-specific option controls, inline validation, and explicit retire/restore actions. Do not offer unavailable type changes.

The document inspector has a Custom fields section with text inputs, multiline editors, a three-state boolean control, exact numeric text inputs, date picker, currency plus amount, select menu, safe URL links, and an accessible document picker. Empty and false must look different. Preserve draft input when a save fails and explain edit conflicts. Link navigation stays within the selected vault and opens the logical document's current revision. Trash targets show a clear retained-link state.

Reuse web form components, spacing, typography, and colors from design/tokens.json and corresponding FolioTokens. Apple uses the existing scoped service and workspace navigation. Do not add colors or spacing literals. Viewers see values and links without mutation controls. Test narrow layouts, keyboard navigation, and accessible labels.

## Workflow events

Reuse the existing durable outbox and foundation's versioned event envelope. Required envelope data includes event ID, occurrence time, vault ID, subject type and ID, nullable server-derived actor, and correlation/causation identifiers. Never create a second event bus.

Stable event names are `metadata_field.created`, `metadata_field.updated`, `metadata_field.deleted`, `metadata_field.restored`, corresponding `metadata_option.*` lifecycle events, and `document.metadata.changed`. Field and option events are non-document events. Value changes target a logical document ID and identify changed field IDs, kinds, and authorized typed before/after values. Clearing values during target purge emits the same value-change event for affected source documents, with system attribution and purge causation. Payloads capture immutable transition data and never require reading a deleted subject later.

Publish outbox records in the same transaction as the mutation. Rejected patches, rollback, unchanged canonical values, repeated retirement/restoration, and retries of a committed request emit no duplicate transition. A single atomic document patch emits one value-change event containing its changed fields. Workflow writes propagate correlation/causation and use the shared recursion guard. Retired definition values remain readable under existing vault access rules. Reject a document move with retained values or incoming links before any metadata event is committed.

The workflow slice consumes these names and payloads using the foundation envelope. Document actions are only valid for `document.metadata.changed`; non-document trigger/action combinations must be rejected explicitly. Both web and Apple workflow trigger selectors display understandable labels, such as Field created, Field retired, Option renamed, and Document custom fields changed. Conditions expose field ID, kind, changed-state, and typed value data only to authorized workflow owners in the event vault. Coordinate trigger integration as a dependency; metadata completion includes verified selectable triggers in both clients.

### Shared event API dependency

Foundation provides `folio_core::domain_event::DomainEventEnvelope`, `DomainEventSubject { subject_type, id }`, and `EventAudience`. Metadata events use `EventAudience::Vault`. Construct the envelope with `new(event_type, vault_id, subject, document_id, audience, data)` and propagate lineage with `with_correlation(correlation_id, causation_id)`. Typed metadata payload structs serialize to the envelope's JSON object; this does not replace typed value persistence.

Append through `folio_infra::database::{sqlite,postgres}::insert_domain_event(&mut Connection, &DomainEventEnvelope)` inside the mutation transaction. Preserve event identity across retries. Foundation owns the `20261102160050_domain_event_identity.sql` migration and legacy event compatibility. Do not duplicate its event identity column or envelope. Verify these proposed signatures against merged source before integration.

Downstream integration must preserve merged personal-token access changes in PR #251 and delayed-event snapshot fixes in PR #252. Rebase onto the merged prerequisite rather than copying its old migration or replacing the current outbox implementation.

The merged envelope requires the first event-name component to match the subject type. Field events therefore use subject type `metadata_field`; option events use `metadata_option`. This replaces the draft dotted names before any metadata producer ships.
