# Processing outcome integration contract

The domain lifecycle envelope merged in PR #265. Integrate the shared workflow registry and revision dispatch contracts as they land. This slice owns processing producers and stage-specific registrations, not those shared contracts.

## Identity and transitions

A processing run has a UUID, document ID, explicit vault ID, immutable revision ID when available, and stage: `ocr`, `embedding`, or `thumbnail`. Reprocessing creates a new run. Queue retries retain the run and increase the attempt number. Each attempt uses the queue claim token as a fence. Events use the foundation envelope and carry payload version 1, run ID, revision ID, stage, attempt, outcome, and an optional allowlisted reason code. Background actor is null. Initiating correlation and causation are retained.

Event names use `document.processing.{stage}.{outcome}`, where stage is `ocr`, `embedding`, or `thumbnail`, and outcome is `started`, `completed`, `failed`, `skipped`, or `retry_scheduled`. Each stage/outcome pair has its own readable registry label, such as OCR completed or Embedding failed. A failure event means the run exhausted retries or encountered a nonretryable failure. A transient attempt failure emits retry_scheduled with its sanitized reason and scheduled retry time. Lease recovery records one retry transition, not heartbeat events. Completed, skipped, and failed are terminal within a run.

Disabled providers produce skipped/provider_disabled. Empty OCR output is a valid OCR completion with has_text=false; embedding skips with no_text. Unsupported thumbnail formats skip with unsupported_format. Provider failures expose stable codes and fixed safe descriptions, never raw provider responses, source text, URLs, credentials, or filesystem paths.

## Atomic writes and stale work

Database operations validate live document ownership, current revision, active run, claim token, and unexpired lease in the transaction that applies state and appends the event. An invalid fence causes no mutation and no event. Duplicate terminal commits are no-ops. Retry decisions, attempt state, and outbox writes share the queue transaction. SQLite and PostgreSQL implement the same contract.

External computation and object uploads precede the transaction. Immutable revision/run-specific object keys prevent late workers from overwriting current results. Only committed references become visible. A completion event means the result reference and derived state committed, not merely that a provider returned. Revision supersession, trash, purge, or vault move invalidate old work. Fresh work in a new vault receives new scoped identity; old events retain their original audience. No cross-vault data enters event payloads.

## Client and workflow behavior

The shared registry exposes labels such as OCR completed, Embedding failed, and Thumbnail skipped. Conditions select stage, terminal outcome, and allowlisted reason. Retry transitions are opt-in and clearly labeled; ordinary failure triggers never fire for an attempt that will retry. Existing workflow authorization and source-event uniqueness apply at execution time. Legacy OcrComplete and ThumbnailReady mapping must not create duplicate effects alongside new events.

JSON document detail adds read-only stage run summaries with revision, attempt, state, reason, and retry time. OpenAPI regeneration supplies the Swift client. HTML and native detail display the same states, distinguish Waiting to retry from Failed, and retain existing reprocessing authorization. Styling uses shared design tokens. No generic event dashboard is added.

## Dependency boundary

Foundation supplies transactional envelope serialization and outbox insertion. Revision owner supplies immutable revision IDs, current-revision fencing, and revision-specific derived storage. Workflow owner supplies registry and builder extension points. Exact Rust signatures and migration numbers are selected from merged implementations rather than duplicated here.

The merged foundation API uses `folio_core::domain_event::DomainEventEnvelope` with `DomainEventSubject { subject_type, id }` and `EventAudience::Vault`. Processing subjects are logical documents with an explicit document ID. Construct the envelope through `new(event_type, vault_id, subject, document_id, audience, data)` so actor, time, and event UUID remain server-derived. Envelope construction reads trusted `mutation::MutationLineage` through `with_lineage`. Persist that lineage with queued work and restore it under system actor scope during retries. `with_correlation(correlation_id, causation_id)` supports explicit producer lineage.

Each adapter calls its foundation `insert_domain_event(&mut Connection, &DomainEventEnvelope)` inside the fenced processing mutation transaction. Do not add another outbox identity migration: foundation owns `20261102160050_domain_event_identity.sql` and its nullable unique event_id. Delivery retries reuse the persisted envelope. Transition retries consult committed run state before allocating any new event.

Consumers must handle `OutboxEvent.envelope: Option<DomainEventEnvelope>` and `event: Option<DocumentEvent>`. New document events can carry both representations; the shared registry chooses one authoritative trigger mapping to avoid duplicate execution. Legacy rows retain their existing compatibility path. These envelope and outbox signatures are merged.

Processing migrations reserve versions `20261102170000` through `20261102170999`, using valid timestamp values and matching SQLite/PostgreSQL versions. Check merged migrations for collisions before assigning a version. Foundation reserves `20261102160050` and `20261102160100` through `20261102160400`. Add processing migrations only after prerequisite schemas exist; test fresh creation and upgrades with independent dependency PRs in different landing orders. Never renumber merged or applied migrations, or create empty reservation migrations.

Integration must preserve scoped personal tokens from PR #251 and the delayed-event snapshot fix from PR #252. The merged personal-token migration owns version `20261102160000`; never rename it. Foundation moved only its unpublished event identity migration to `20261102160050`. Verify these prerequisites against merged main before rebasing processing implementation.

## Revision adapter boundary

The revision owner proposes `document_revisions(id, document_id, vault_id, revision_number, original_file_id, created_at, created_by, restored_from_revision_id)` and `documents.current_revision_id`, nullable until finalization. Stage runs may retain a null revision ID for legacy processing until revision persistence lands. Never manufacture a revision or infer identity from a storage key. Once a job has a revision, its run retains that identity through every retry.

Preserve legacy `JobType::ProcessDocument` decoding. The revision owner supplies `ProcessDocumentRevision { document_id, revision_id }` and `ProcessingQueue::enqueue_revision_processing(document_id, revision_id)`, including transactional current-revision resolution for legacy enqueue requests. Stage result transactions compare the expected revision, current vault, active run, and job claim before publishing current state or events. The revision owner owns revision rows and dispatch conversion; this slice owns stage transitions.

Merged envelope APIs include `insert_domain_event_with_origin` and `insert_event_with_envelope` in both database adapters. Preserve explicit workflow origin and lineage. Use the combined projection/envelope insertion when a legacy document event is required, rather than emitting a second event that can trigger duplicate effects.

## Workflow catalog integration

Register producer definitions through `WorkflowTriggerRegistry` using `WorkflowTarget::Document`. Every definition uses subject_type=document. Condition fields include `stage`, `attempt`, `reason_code`, `revision_id`, `run_id`, and the bounded immutable document occurrence fields required by the shared engine. Completion may add `has_text` for OCR. No OCR text, provider body, local path, or credential enters the occurrence snapshot. Preserve mutation ID across a canonical stage event and its compatibility projection; do not equate mutation ID with the longer-lived run or correlation ID.

Stage attempt numbers count starts of that stage within a run. Queue attempts may include failures before a stage starts, so adapters retain the queue claim and queue attempt separately for fencing. Restoring persisted stage state does not replay attempts or emit events.
