# Document discussion design

## Ownership and lifecycle

A document entry has `id`, `document_id`, `vault_id`, `kind`, `body`, `created_at`, `updated_at`, `deleted_at`, `created_by`, and `updated_by`. Kind is `note` or `comment` and cannot change. Attach entries to the logical document ID, never a file version. Creation sets both timestamps and both actors together. Updates preserve creation metadata and replace update metadata. The server derives audit actors from the request. Actor IDs use the existing nullable string UserId representation. Unknown legacy or system actors remain null and display as Unknown author.

A single repository port and table serve both kinds. Reads exclude deleted entries. Deletion retains the body and attribution for lifecycle consistency but removes the entry from ordinary APIs and UI. Trashing a document hides its entries with the document; restoring the document preserves surviving entries. Permanent document deletion removes its dependent entries. Document moves must carry entry vault ownership atomically, preserving creation metadata and recording the mover as update actor, without cross-vault leakage. Integrate this into the existing document move transaction after foundation merges. Keep immutable document audit and outbox payloads unchanged.

## Permissions

All reads require current vault membership and an accessible live document. Viewers cannot create entries. Contributors and higher roles can add entries. Only an entry author can edit its body while retaining a writable role. Authors can delete their own entries; editors and owners can delete others' entries for moderation. Editors cannot rewrite another author's words. Null authors confer no author privilege. Clients receive explicit edit/delete capabilities and never infer permission from display names.

Authorization and mutation must remain valid at the write boundary. Entry lookup binds both document and vault. A guessed ID from another document or vault returns not found. Revoked access applies to subsequent pages and writes.

## API and pagination

Use `/api/documents/{id}/entries` for GET and POST, and `/api/documents/{id}/entries/{entry_id}` for PATCH and DELETE. GET requires kind and accepts a bounded limit and continuation. Each continuation binds document, vault, and kind. Order by `(created_at, id)` ascending, with one extra record to determine continuation. Updates do not reorder entries. Clients deduplicate IDs between pages and refresh after mutation.

Accept plain text with non-whitespace content and a 16 KiB UTF-8 bound. Escape text in HTML and render literal text in SwiftUI. Creation returns 201, editing returns 200, and deletion returns 204. Malformed bodies/cursors return 400; unauthenticated requests use existing authentication behavior; disallowed visible actions return 403; invisible or missing parents/entries return 404. Do not accept lifecycle fields from input.

The HTML routes preserve the selected vault, use existing CSRF protection, and redirect to the relevant document section. Generate `openapi/openapi.json` and compile the Swift client from its linked schema input.

## Interface

Use the existing document detail and shared spacing, typography, and color tokens. Provide Notes and Comments sections with short explanatory empty states. Each entry shows author, creation time, and an Edited indicator when updated. Preserve multiline text. Expose Add note and Add comment separately, with labeled editors, Save and Cancel controls, and a delete confirmation. Show only authorized actions.

Apple opens the discussion in a sheet from document details so the preview remains usable on compact screens. Both clients support loading more entries, retrying failed reads, and retaining unsaved text after failed writes. Use accessible labels, keyboard-reachable controls, and error announcements. Avoid adding an unrelated discussion navigation destination.

## Workflow events

Publish `document.note.created`, `document.note.updated`, `document.note.deleted`, `document.comment.created`, `document.comment.updated`, and `document.comment.deleted` through the existing durable outbox in the entry mutation transaction. These are document-targeted events with entry identity as the subject and the logical document as the action target. Entry restoration is not exposed by this change. Document restoration does not fabricate entry creation events.

Use foundation DomainEventEnvelope schema version 1 with schema_version, event_id, event_type, occurred_at, vault_id, subject, actor_id, correlation_id, causation_id, document_id, and allowlisted data. Subject type is document_note or document_comment, with the entry ID; document_id is the logical document ID. Event audience is Vault. Event data includes entry_kind and transition. Preserve the monotonic outbox row ID as the delivery cursor. Event condition data includes entry kind and transition, entry ID, and document identity. Do not include entry bodies in broadcasts or workflow payloads. Recheck the workflow owner's access to the event vault and document before execution, including after document moves or membership revocation. Deletion retains only the immutable event metadata needed for conditions, without requiring the entry body to remain readable.

No-op edits and repeated deletion emit no events. Rollback emits no events. A mutation retry must reuse its idempotency identity and cannot create a duplicate entry or event. Consumers retain existing outbox delivery deduplication and workflow-origin recursion protection. Preserve compatibility with historical document event payloads.

Expose these six triggers with readable Notes and Comments labels and supported condition fields in both workflow editors. Reject actions that require an unavailable live document. Coordinate shared envelope and trigger registry integration with the foundation and workflow slices before merging.

Use `folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience}` and each backend’s `insert_domain_event(&mut Connection, &DomainEventEnvelope)` inside the entry transaction. The constructor derives actor, time, and event UUID; `with_correlation` preserves lineage. Reuse foundation migration `20261102160050_domain_event_identity.sql` for event identity deduplication. Do not add a second outbox identity migration. Verify final signatures after foundation merges.
