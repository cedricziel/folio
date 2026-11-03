## Data and ownership

Use the validated `DocumentDate` wrapper around `chrono::NaiveDate` for optional `document_date`, serialized as YYYY-MM-DD. Accept years 1 through 9999 and strict ASCII calendar dates in both Rust and Swift. SQLite stores canonical text and PostgreSQL stores DATE. Do not translate dates through a local timezone or change `created_at` when editing them. OpenAPI uses inline nullable string/date fields for Swift generator compatibility. A custom generated-client PATCH payload preserves omitted fields, explicit null, and date strings; synthesized optional encoding cannot represent all three.

Review fields are `reviewed_at`, nullable `reviewed_by`, and nullable `review_origin` with human, system, or legacy_classification values. Reviewed means reviewed_at is present. Unreviewed clears all three fields. Signed-in review uses the authenticated actor and server clock. Authentication-disabled review uses system provenance with no actor. Request bodies never accept actor, timestamp, or origin. Repeated requests for the current state preserve attribution and timestamps.

At migration time, legacy records that satisfy the current Filed rule receive a migration timestamp, legacy_classification provenance, and no reviewer. Other records stay unreviewed. The timestamp means migration, not a historical human decision. Display this distinction in detail views. New uploads, including automatically classified imports, start unreviewed. OCR, classification, and metadata edits preserve review decisions. File replacement will clear review in the later revision change.

Follow the domain-lifecycle contract: created_at/updated_at/deleted_at are top-level UTC timestamps, created_by/updated_by are nullable UserId strings, and vault_id is required on vault-owned responses. Mutation DTOs exclude lifecycle and ownership fields. Keep uploaded_by for contributor authorization. Use foundation actor context for updated_by and existing access policy for edits. Trash forbids metadata/review mutations until restore. Review state survives trash and restore.

## API and queries

Add PATCH `/api/documents/{id}` for document_date, preserving the repository's omission-versus-null convention. An omitted date is unchanged; null clears it. Reject impossible dates, timestamp strings, and unsupported fields with the existing JSON extractor’s 422 validation response. Add idempotent PUT `/api/documents/{id}/review` with a required boolean `reviewed`; return the updated document with 200. Provide vault-scoped equivalents through the existing scoped router.

Read and browse DTOs expose these fields. Add reviewed=true/false and inclusive document_date_from/document_date_to filters. Null dates do not match a date bound. Bind all new filters into pagination cursors and reject reversed bounds. Leave existing created_after/created_before semantics intact. Date filtering is unrelated to import-date filtering.

For authorization, use 401 for missing required authentication, 404 for missing/inaccessible documents, and 403 for insufficient roles within accessible vaults. Publish the additive server contract in OpenAPI and regenerate the Swift client using its existing schema input. Use the project's explicit server compatibility mechanism before exposing unavailable native actions.

## Presentation

Web and Apple detail show Document date separately from Imported. Provide a validated YYYY-MM-DD date field, Clear date, and Mark reviewed/Mark unreviewed with pending and retryable error states. Review badges identify legacy migration or the human reviewer when available. Both browsers expose review state and document date bounds. Inbox uses explicit unreviewed state; processing progress and failed-processing discovery remain independent. Keep the current document selected after edits. Use existing tokens and controls.

## Delivery

Ship a complete date/review PR with both clients after foundation merge. If it needs smaller PRs, land date editing and then review, with each PR including its own usable web and Apple controls. Do not call the slice complete after backend delivery.

## Workflow event contract

Reuse the foundation's versioned envelope and existing durable outbox. All events here target subject_type=document and subject_id=document_id, with event identity, occurred_at, vault_id, nullable server actor, and inherited correlation/causation/origin. Do not create another event bus or regenerate event identity during delivery retries.

| Event | Trigger label | Version 1 condition data |
| --- | --- | --- |
| document.date_changed | Document date changed | previous_document_date, document_date |
| document.reviewed | Document marked reviewed | reviewed_at, reviewed_by, review_origin |
| document.unreviewed | Document marked unreviewed | previous_reviewed_at, previous_reviewed_by, previous_review_origin, reason |

Dates use YYYY-MM-DD or null. Review timestamps use UTC RFC 3339. Unreview reason is user, replacement, or restoration. Capture condition data and the existing bounded document metadata snapshot inside the mutation transaction. Classification and processing never emit review transitions. Legacy migration does not trigger user workflows.

Publish each actual transition exactly once in the same database transaction. Identical date writes and unchanged review requests emit no event. Rollbacks emit none. Preserve the existing document.updated subscription semantics for metadata changes; consumers deduplicate by event identity and use the specific trigger for the specific transition. Never allow workflow-origin mutations to recursively trigger workflows without the foundation's recursion guard. A moved/deleted subject does not regain access through its stored event snapshot; recheck vault and subject policy before actions.

Expose these triggers and their condition fields with the labels above in both workflow editors. The workflow slice owns the shared selector/condition engine; this slice owns event definitions, transactional emission, and regression coverage, and verifies the integrated selectors before completion.

## Shared document update boundary

The document-transition slice owns the common before/after delta helper and title, classification, tag, trash, restore, purge, and move event producers. This slice extends UpdateDocument with optional nullable document_date and composes its date delta at that same transactional boundary. Preserve generic Updated compatibility once per actual metadata mutation; add document.date_changed only when the date differs. No-op detection precedes timestamp/audit updates and event insertion.

Review uses a dedicated repository operation with a server-derived actor and boolean target state, rather than accepting review attribution in UpdateDocument. Revision append and restore use separate conditional operations. Those operations own their specific events and any resulting unreview event. All event inserts share the domain mutation transaction and foundation envelope. Coordinate adapter integration after foundation merge; do not replace the document-transition helper with an independent event producer.

## Foundation API integration

Foundation envelope PR #265, trusted lineage PR #266, and metadata producer PR #271 are merged. Date integration uses these contracts:

- `folio_core::domain_event::DomainEventEnvelope::new(event_type, vault_id, subject, document_id, audience, data)` derives actor, time, and UUID on the server.
- `DomainEventSubject { subject_type, id }` serializes subject_type as `type`. These events use document subject type and `Some(document_id)`.
- `EventAudience::Vault` applies to this slice's document events. Consumers still recheck current vault/document access. Do not use Owners or User audience to bypass document authorization.
- `with_correlation(correlation_id, causation_id)` carries workflow lineage. Reuse the envelope identity across retries instead of calling new again for delivery.
- `folio_infra::database::{sqlite,postgres}::insert_domain_event(&mut Connection, &DomainEventEnvelope) -> Result<i64>` must execute on the domain mutation's transaction connection.

The foundation owns migration `20261102160050_domain_event_identity.sql` and its nullable unique event_id. Do not add a parallel envelope table, identity migration, or event bus. Payload data schemas remain owned by this slice. OutboxEvent exposes optional envelope and optional legacy DocumentEvent; specific date events carry an envelope, and the single generic Updated row carries both the envelope and legacy event. Old rows may have only the legacy event. Keep current SSE/workflow consumers compatible and test both row formats.

## Migration reservation

This slice owns migration versions 20261102161000 through 20261102161999. Use valid timestamp-style versions with matching SQLite and PostgreSQL files. Before creating a migration, check merged main for unrelated collisions. Only unpublished, unapplied task-owned migrations may be renumbered. Verify fresh ordering and upgrades with dependency PRs landing in different orders. This reservation does not change the foundation merge gate.

The foundation prerequisite migration is now 20261102160050_domain_event_identity.sql because merged personal-token migration 20261102160000 already owns the earlier version. Preserve scoped-token PR #251 and delayed-event snapshot PR #252 when integrating foundation. Do not rename the merged migration.
