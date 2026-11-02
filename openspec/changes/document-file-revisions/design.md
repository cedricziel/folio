## Logical documents and immutable files

Documents retain IDs, metadata, tags, links, and notes. Add current_revision_id. A revision has id, document_id, explicit vault_id, monotonically increasing number, original file reference, created_at, nullable server-derived created_by, and optional restored_from_revision_id. Revisions are immutable history and therefore deliberately omit mutable updated_at/updated_by/deleted_at fields. Document soft deletion controls their visibility. Record this exception alongside the foundation's immutable-event rules.

A stored file records SHA-256 checksum, byte size, MIME type, opaque storage key, and role. Original and retained archival-derivative bytes have separate identities. Never use client assertions for checksum or size. Hash actual bytes on every upload path. Use application/octet-stream when MIME cannot be determined; MIME describes the file, not trust in its contents. Revision-local keys include a random revision ID, so concurrent writes cannot target the same object. Do not deduplicate across vaults.

Once published, original and archival bytes and their identity rows cannot be updated. Preview caches are regenerable and revision-scoped; retained archival outputs are append-only with their source revision and generation identity. Retrying a processor must either reuse a verified identical output or record a new output, never overwrite a retained artifact.

## Commit and concurrency

Replacement requires expected_current_revision_id and an idempotency key. Store bytes under a fresh revision key, verify identity, then atomically check the expected pointer, allocate the next number, insert revision/file rows, switch current_revision_id, clear review, reset current processing projection, and enqueue a revision-bound job/outbox event. Enforce unique(document_id, number) and a per-document idempotency key in both databases. Lock or compare-and-swap the document inside the transaction. Failed storage never publishes a revision. A losing compare-and-swap returns 409 and removes only its own unreferenced staged objects. Retrying a successful idempotency key returns the same revision; different content under that key returns 409.

A restoration appends a new numbered revision referencing the verified immutable original of the chosen historical revision. Record restored_from_revision_id and the restoring actor. Restoration never rolls back or edits history. It uses the same conditional current-pointer update as replacement and clears review. Metadata, tags, links, and notes remain on the logical document.

## Processing and search

Jobs capture revision_id, not just document_id. OCR output, chunks, embeddings, page count, and thumbnails carry that identity. Work for revision N may finish after N+1 becomes current, but it cannot update N+1's projection, publish misleading current events, or enter current search results. Every write that updates the document projection checks current_revision_id transactionally. Search joins only current, live revisions. Reprocessing the same file preserves human review. Replacement and restoration clear review because the presented bytes change.

## Migration

SQL migrations create revision 1 for each legacy stored original, preserving the original key and document identity. Metadata-only or incomplete uploads have no revision until finalization. Legacy identities start in explicit pending verification state with unavailable checksum/size, rather than fabricated values. A resumable backfill streams bytes, computes checksum/size, and marks identity verified atomically. Missing/corrupt objects produce visible unavailable/error state and retries; migration never silently substitutes bytes. Historical metadata/downloads remain usable when existing bytes are present. Replacement and restore require verified identity for their source; new uploads never publish pending identity. Backfill must be restartable and scoped so trash/purge races cannot resurrect rows.

## Authorization and lifecycle

History and historical downloads use the same vault and document access policy as current downloads. Current and historical content are hidden by trash until restore; purge removes every revision, derived output, processing result, and object owned exclusively by that document. Purge cannot race a late upload into orphaned retained bytes. Shared original references from restore survive until the entire logical document is purged.

Move the logical document and all file, revision, chunk, and artifact ownership in one authorized transaction. IDs and storage keys remain stable. Preserve creation attribution, attribute the update to the mover, remap live classifications by name, and write audit events to both vaults as required by the foundation. Recheck source/destination access and taxonomy compatibility using foundation semantics. Jobs use current document ownership or are fenced; a stale source-vault worker cannot write after the move. No historical download may authorize against an old vault_id.

## API and UI

Add GET `/api/documents/{id}/revisions` for bounded history; POST at that path for multipart replacement; GET `/api/documents/{id}/revisions/{revision_id}/download`; and POST `/api/documents/{id}/revisions/{revision_id}/restore`. Support equivalent vault-scoped routes. Reads return 200, new revisions 201, malformed requests 400, unauthenticated 401, insufficient accessible-vault role 403, missing/inaccessible 404, stale current revision or conflicting idempotency key 409, and oversized payload 413. Document original download resolves current_revision_id; old logical links continue to work. Update OpenAPI, generated Swift client, and MCP current-original resources together.

Web and Apple detail display current revision, original filename, size, MIME, and verification status. A history list shows number, actor/time, restore provenance, and download actions. Replacement uses a file picker with pending state, retryable errors, and conflict refresh. Restore confirms that a new revision will be created. Refresh preview caches by account/server/vault/document/revision identity. Use existing tokens and preserve browse selection.

## Delivery sequence

After foundation and date/review merge, first ship durable identity/backfill with file details in both clients. Next ship complete immutable revisions, history, replacement, and restoration with both clients. Keep the importer and processing conversion coherent: never enable replacement while workers can overwrite unversioned current content. Split further only when every intermediate PR remains independently safe and usable.

## Workflow event contract

Use the same foundation envelope and durable outbox as date/review events. The subject is the logical document, not a non-document file object. Version 1 payloads contain revision_id, revision_number, previous_revision_id, original filename, MIME, byte size, checksum, and nullable restored_from_revision_id. Do not expose storage credentials, signed URLs, original bytes, or OCR content.

| Event | Trigger label | Emission |
| --- | --- | --- |
| document.revision_created | Document file revision created | Every newly published revision, including first upload and restore |
| document.revision_restored | Document file revision restored | A new revision created from a historical revision |

First uploads retain document.uploaded compatibility. Replacement retains existing document.updated compatibility. Restore emits revision_created and revision_restored with separate event identities and shared correlation, so each selectable trigger has an exact meaning. Clearing an existing review during replacement or restore emits document.unreviewed with the corresponding reason in the same transaction. A replayed idempotency key emits nothing new. Failed uploads, rejected stale writes, backfills, and rollback emit nothing.

The event envelope and bounded condition snapshot are captured transactionally. Delivery retries retain event identity. Inherit workflow origin and causation to enforce recursion protection. Recheck subject access and current ownership before workflow actions; immutable snapshots do not grant access to moved or deleted files. Foundation trash/restore/delete/move events remain authoritative for logical lifecycle. Expose the new labels and file condition fields in web and Apple workflow selectors through the shared workflow slice.

## Foundation API integration

The proposed foundation API is not merged. Verify these signatures against its merged commit before implementation:

- `folio_core::domain_event::DomainEventEnvelope::new(event_type, vault_id, subject, document_id, audience, data)` derives actor, time, and UUID on the server.
- `DomainEventSubject { subject_type, id }` serializes subject_type as `type`. These events use document subject type and `Some(document_id)`.
- `EventAudience::Vault` applies to this slice's document events. Consumers still recheck current vault/document access. Do not use Owners or User audience to bypass document authorization.
- `with_correlation(correlation_id, causation_id)` carries workflow lineage. Reuse the envelope identity across retries instead of calling new again for delivery.
- `folio_infra::database::{sqlite,postgres}::insert_domain_event(&mut Connection, &DomainEventEnvelope) -> Result<i64>` must execute on the domain mutation's transaction connection.

The foundation owns migration `20261102160050_domain_event_identity.sql` and its nullable unique event_id. Do not add a parallel envelope table, identity migration, or event bus. Payload data schemas remain owned by this slice. OutboxEvent exposes optional envelope and optional legacy DocumentEvent; new document events carry both through foundation compatibility mapping, while old rows may have only the legacy event. Keep current SSE/workflow consumers compatible and test both row formats.

## Migration reservation

This slice owns migration versions 20261102161000 through 20261102161999. Use valid timestamp-style versions with matching SQLite and PostgreSQL files. Before creating a migration, check merged main for unrelated collisions. Only unpublished, unapplied task-owned migrations may be renumbered. Verify fresh ordering and upgrades with dependency PRs landing in different orders. This reservation does not change the foundation merge gate.
