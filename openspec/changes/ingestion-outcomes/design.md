# Ingestion outcome contract

## Shared dependencies

Use foundation `DomainEventEnvelope` version 1 and the existing transactional outbox. Keep integer delivery cursor and UUID event identity. This slice does not create a bus or change source CRUD events. Foundation contract is `openspec/changes/domain-lifecycle/design.md`. Workflow triggers come from the typed registry merged in #280 (`openspec/changes/archive/2026-11-03-workflow-trigger-catalog`).

## Events and payloads

| Event | Label | Subject | Data |
| --- | --- | --- | --- |
| ingest.imported | Item imported | ingest | payload_version, source_id, source_kind, item_id, attempt_id, outcome, reason_code |
| ingest.skipped | Item skipped by a rule | ingest | same fields |
| ingest.duplicate | Duplicate item found | ingest | same fields |
| ingest.failed | Item import failed | ingest | same fields, retryable |
| source.failed | Source became unavailable | source | payload_version, source_id, source_kind, reason_code |
| source.recovered | Source recovered | source | payload_version, source_id, source_kind |

All producer payloads contain `payload_version: 1`, independently of the envelope schema version. All events use resolved vault ownership, nullable system actor, server occurrence time, shared correlation/causation, and schema version 1. The persisted imported outcome can retain a confirmed document link for authorized UI reads. All six event envelopes have no document target because their subjects are ingestion items or sources. They never expose a reserved future document ID. Initial healthy polls emit nothing. Initial failure emits once; repeated failure, including changed error text, emits nothing until recovery. Successful polls with item failures are reachable sources, not source failures.

Reason codes come from a fixed enum such as rule_skipped, content_duplicate, import_failed, source_unavailable. No arbitrary error text, rule names, file paths, hashes, message IDs, filenames, mail headers, raw messages, credentials, or source configuration enter the payload. User-facing text maps codes to readable explanations. Preserve existing internal errors only where already required; new outcome UI never renders arbitrary provider errors.

## Item identity versus content deduplication

The existing content ledger prevents duplicate documents within a vault. Keep it separate from source item identity. Persist a UUID item identity behind a unique vault/source/opaque locator key. Folder locators identify a file generation using path and stable observed file metadata/content identity. Email locators identify mailbox UIDVALIDITY/UID and attachment position, including the optional message body. Store only a digest of the canonical locator. Do not expose locators in events or APIs.

A new observation of the same unchanged item reuses item identity. Busy content ownership and repeated completed observations are not attempts and publish nothing. A duplicate is emitted once when a distinct source item first encounters retained content. Skipped items emit once for the same evaluated decision; polling must still permit a changed pre-import rule to admit the item later. Persist a bounded decision fingerprint for that purpose without publishing the rule text.

An actual claim starts a UUID attempt. Lease renewal after expiry or explicit retry starts a new attempt; the previous attempt is fenced. At most one terminal outcome exists per attempt. A stale completion cannot publish. Existing `created_at` microsecond fencing remains effective during migration. Recovering a completed import after a crash records the reserved document identity without importing another document.

## Atomicity and recovery

Commit terminal outcome, operational observation state, any imported/retryable content-ledger transition, and outbox append in one database transaction. `IngestOutcomeRepository::complete` receives the fenced content reservation when an import was attempted; it must not publish after a stale content claim. This prevents a crash between ledger finalization and outcome publication from converting a successful import into a duplicate outcome on replay. Use unique attempt terminal records and conditional updates for concurrency. Publisher failure rolls back all three. Replayed completion is a no-op. A durable imported outcome is only recorded after original storage and document persistence are confirmed. A failed import retains retryability and does not acknowledge the source. Source health comparison, status persistence, and event append share a transaction with row locking or equivalent conditional serialization.

Source status writes receive the resolved vault and source kind, including configuration sources. Recheck managed-source ownership/lifecycle before publication. No backfill emits events. Immutable outcomes do not gain mutable lifecycle fields. Operational claims retain their lease rules.

## UX and compatibility

Source detail shows recent outcome time, readable result, retry state, and a document link only when authorized and present. Empty, loading, failed-load, and retry states exist on both web and Apple. Show source unavailable/recovered state without raw email diagnostics. Use shared design tokens.

Add `GET /api/sources/{id}/outcomes` with `before` and `limit` query parameters, default limit 50 and maximum 100. The response contains vault_id, source_id, outcomes, and nullable next_before. Use the existing X-Folio-Vault selection and source-management permission. Add `GET /settings/sources/{id}/outcomes` for the web page. Update OpenAPI and regenerate Swift. Keep credentials redacted. Shared workflow registry exposes the six labels and source ID/kind, outcome, reason, and retryability conditions. Outcome triggers cannot use SkipImport; it remains synchronous on ingest.received. Register all six triggers as non-document triggers; imported document lifecycle events already provide document automation. Recheck current authorization before any action and retain original event context.

## Integration blockers

Actor context, ownership, transactional envelopes/outbox, and trusted lineage are merged in #258, #260, #265, and #266. Stable source item identity is merged in #269. Workflow registry types and non-document action compatibility still require integration with the workflow owner. Do not introduce parallel interfaces.

## Landed foundation API alignment

The outcome worktree is based on merged main through #269. The implemented signatures are:

- `folio_core::domain_event::{DomainEventEnvelope, DomainEventSubject, EventAudience}` supplies the only envelope.
- `DomainEventSubject { subject_type, id }` serializes subject_type as `type`.
- `DomainEventEnvelope::new(event_type, vault_id, subject, None, audience, data)` derives server actor, occurrence time, and UUID identity. Data is an allowlisted JSON object.
- `with_mutation(mutation_id)` groups events from one committed transition; correlation may span multiple mutations and must not suppress later transitions. `with_correlation(correlation_id, causation_id)` retains incoming lineage. Background poll roots establish system context; detached tasks must not inherit an unrelated request actor.
- `folio_infra::database::{sqlite,postgres}::insert_domain_event(&mut Connection, &DomainEventEnvelope) -> Result<i64>` runs on the same connection and transaction as the outcome or health mutation.
- `OutboxEvent.envelope` is optional and `OutboxEvent.event` becomes optional. These six non-document events carry the envelope with no legacy DocumentEvent.
- Foundation migration `20261102160050_domain_event_identity.sql` owns nullable unique event_id. The ingestion migration must not duplicate envelope or outbox identity storage.

All six ingestion producers deliberately use no document target, including ingest.imported. An authorized outcome API can link its persisted document separately. Consumers retain event context and reject document-only actions.

Use `EventAudience::Owners` for all six events. Current `vault::action` requires `ManageWorkflows` for source reads, which `VaultRole` grants only to owners. Outcome APIs retain that permission. Never use actor attribution as authorization. Current ownership, source access, and vault lifecycle are rechecked before matching or delivery; deleted vaults execute no workflows.

Create an envelope once for a transition and retain it across transaction retries. Replayed committed completion returns the stored outcome and event identity without constructing a replacement event. Outbox dispatch retries reuse persisted envelope bytes. Unique attempt completion and conditional health transitions prevent duplicate events even across concurrent producers; UUID deduplication alone does not replace those guards.

## Migration allocation

This slice reserves versions 20261102171000–20261102171999. Use valid timestamp values within that range, starting with `20261102171000_ingestion_outcomes.sql`, and identical versions in SQLite and PostgreSQL. Foundation owns `20261102160050` and `20261102160100` through `20261102160400`. Check merged main for collisions and concrete prerequisites before creating migrations. Never renumber merged or applied migrations. Fresh database and upgrade tests must cover dependency landing order; the reservation does not permit merging before prerequisites.
