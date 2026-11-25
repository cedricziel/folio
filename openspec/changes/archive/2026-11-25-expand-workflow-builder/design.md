## Existing behavior

WorkflowEngine consumes durable outbox events and ignores Workflow origin. Runs freeze actions and event payloads and persist completed action counts. The current web form replaces the first action of each supported kind. Apple has no workflow views. No outbound email delivery configuration was found in crates.

## Definition and compatibility

Keep existing wire names and accept legacy single-trigger input. Normalize definitions to a nonempty, duplicate-free trigger list. All selected triggers must support every action. IngestReceived permits only SkipImport, and deletion permits only delivery actions. Reject incompatible combinations rather than silently omitting actions.

Use bounded condition groups with all, any, and none semantics. Empty all and none match; empty any does not. Missing metadata never satisfies a typed comparison. Validate field types and references in the workflow vault. Keep legacy conditions as a conjunctive group. Bound depth and predicate count before evaluation.

## Execution

Freeze definition revision, event context, and ordered actions when enqueueing. Before each attempt, check that the definition is active and the execution authority remains valid. Definition edits cancel pending runs from older revisions rather than changing their snapshots. Retain history and explain cancellation. Follow the foundation actor contract; clients cannot choose audit actors.

Document mutations use the normal vault-scoped service path and Workflow event origin. A retry resumes completed steps. Mutations must be idempotent across a crash between mutation and progress persistence. Webhooks retain stable per-action delivery identifiers. Do not claim exactly-once external delivery.

Schedule claims and run creation must share a transaction with a unique occurrence key. Persist the next occurrence and definition revision. Use UTC instants for scheduled and recurring triggers, with explicit date-only semantics for offsets. Define bounded catch-up after downtime and test concurrent schedulers and restart. Editing or disabling a schedule invalidates outstanding claims. Date-offset occurrence keys include the relevant document date so changed dates do not accidentally reuse old occurrences.

## User interfaces

Web and Apple builders expose trigger selection, schedule fields, condition groups, and an ordered action list. Users can add, remove, and move actions. Show compatible choices and server validation beside the relevant fields. Existing definitions must round-trip without dropping fields. Run history includes status, completed actions, cancellation reason, and retry outcomes without secrets.

Use existing navigation and shared design tokens. Add native models and service adapters through the generated OpenAPI client. Provide loading, empty, error, and read-only states. Keep integration credentials separate from document action fields. Reordering a webhook cannot silently transfer stored credentials to a different destination.

## Dependency contracts

The foundation design on `codex/domain-lifecycle` establishes top-level UTC
`created_at`, `updated_at`, and nullable `deleted_at`, nullable string
`created_by` and `updated_by`, and required UUID `vault_id`. Mutation DTOs do not
accept these fields. System workflow actions use null actors and never impersonate
the workflow author or uploader. `uploaded_by` remains an authorization field.
Deleted classifications keep existing references; new assignments reject deleted
or cross-vault targets. Live names are unique, and restore conflicts on reuse.
Workflow run progress is operational state; captured actions and payloads remain
immutable. Integration remains gated on the foundation merge.

- Foundation: explicit vault ownership, lifecycle columns, server-derived audit actors, soft deletion and active-reference lookup.
- Metadata: typed definition/value representation, validation, vault-scoped set/clear services, and immutable event snapshot encoding.
- Document lifecycle: document date semantics, reviewed state mutations, version/review event names, and historical snapshots.

Resolve these contracts from merged code before implementing integrations. Do not duplicate their models or migrations.

### Merged metadata and document producers

Protected main at `54e012a9` contains the typed metadata value repository,
metadata occurrence dispatch in #543, and native metadata authoring in #556.
`MetadataValue` carries a `kind` and typed `value`. Integers, decimals, dates,
and money amounts retain their exact strings. The vault-scoped
`MetadataValueRepository::patch` checks the expected revision and publishes
`document.metadata.changed` in the write transaction. A no-op keeps the
revision and emits no event. Match the committed before and after values.

Both databases use `20261102161000_document_date.sql` and
`20261102161030_document_review.sql`. Reuse the date-only `DocumentDate` type
and the scoped review mutation with server-derived attribution. The document
producer publishes `document.date_changed`, `document.reviewed`, and
`document.unreviewed` with captured transition data. Revision publication in
#552 and #554 uses `document.revision_created` and
`document.revision_restored` with the new and previous revision identities.
Replacement and restoration clear review in the same transaction and retain
their shared mutation identity. The read API in #511 exposes immutable revision
history. The workflow consumer and builder work for these transitions remains
tracked under 3.2 and 3.3.

## Condition group encoding

Extend the existing conditions object with `all` and `none` arrays and an optional
`any` array. Each entry is another conditions object. Existing leaf fields and
all present groups must match together. An omitted `any` does not constrain the
match; an explicit empty `any` never matches. This preserves existing serialized
conditions without a migration. Limit the tree to eight nested groups and 64
condition objects. Validate source patterns at every node before persistence.
Evaluation uses the same tree for stored documents, immutable event snapshots,
and ingestion candidates. The web editor must preserve group fields while its
leaf controls are edited, until group editing controls are available.

## Shared domain event integration

The foundation owns the shared versioned envelope and transactional outbox
publication. Workflows consume it without a separate bus. A trigger registry
maps stable event names to human labels, subject kinds, available condition
fields, and compatible actions. Preserve existing document wire names where
possible. Never invent a document ID for a non-document event. Reject document
actions for non-document triggers, including mixed trigger selections.

Match only events visible to the workflow's execution authority. Private view
and discussion events must not leak through webhook payloads, conditions, or run
history. Resolve this authority with the foundation access contract before
integration. Nullable system attribution does not grant access. Keep vault-move
handling scoped to each committed event's original vault and recheck current
subject access before mutations.

Preserve event identity as the durable run deduplication key and use shared
correlation/causation for recursion protection. Workflow definition mutations
also emit committed creation, update, deletion, and restoration transitions via
the shared publisher. No-op writes and rolled-back writes emit nothing. Run
progress remains operational and does not produce recursive definition events.

### Foundation envelope contract

The merged `DomainEventEnvelope` in #265 and #266 contains `schema_version = 1`, UUID
`event_id`, string `event_type`, UTC `occurred_at`, `vault_id`, typed subject ID,
nullable `actor_id`, UUID `mutation_id` and `correlation_id`, nullable UUID `causation_id`, optional
`document_id`, and immutable allowlisted `data`. Keep the existing integer
outbox cursor and `source_event_id` deduplication. Non-document events never
supply a synthetic document ID.

Foundation defines `EventAudience::{Vault, User { user_id }, Owners}`.
Check event-time audience and current access before matching or delivery,
including deleted subjects. System attribution never grants access. `Owners`
requires current owner access. Workflows in deleted vaults do not execute.

The shared constructor is
`DomainEventEnvelope::new(event_type, vault_id, subject, document_id, audience, data)`.
It derives actor, time, and event identity on the server. The constructor reads task-local lineage, and `with_correlation` sets explicit server-supplied lineage.
Producer adapters call `insert_domain_event(&mut Connection, &envelope)` within
the mutation transaction. Foundation owns migration
`20261102160050_domain_event_identity.sql`; this slice does not duplicate it.
These signatures were verified on protected main at `c0e840b8`. Preserve scoped-token
authorization from PR #251 and immutable delayed-event snapshots from PR #252.
The merged personal-token migration owns version `20261102160000`; never rename
it or reuse that version for domain events.

The merged helper exports are `folio_core::{DomainEventEnvelope,
DomainEventSubject, EventAudience}`. Envelope fields have read-only getters.
`with_mutation(UUID)` and `with_correlation(UUID, Option<UUID>)` return validated
envelopes. SQLite and PostgreSQL expose `insert_domain_event`,
`insert_domain_event_with_origin`, and `insert_event_with_envelope` for the
caller's transaction. Foundation retains workflow soft-delete storage ownership;
this slice adds lifecycle publication after that prerequisite merges.

### Producer registration seam

Producers supply `WorkflowTriggerDefinition` values to a shared
`WorkflowTriggerRegistry`. A definition declares stable `event_type`, a plain
`label`, `subject_type`, a `WorkflowTarget`, and typed `condition_fields`.
`WorkflowTarget` distinguishes `Document`, `Event`, and `IngestCandidate`.
The registry rejects duplicate event names, unknown selections, duplicate
selections, malformed field catalogs, and action combinations unsupported by
any selected trigger. Registration does not imply authorization to consume an
event. Audience and current access checks remain separate execution gates.

The workflow JSON API will expose the registered definitions to both builders.
Producers register only after their transactional publisher and condition data
are integrated. The engine and clients use one catalog; producers do not add
separate web or Apple selection controls. A non-document event with an optional
parent document ID retains `Event` compatibility unless its contract explicitly
guarantees a mutable document target.

### Workflow event ownership

This slice owns definition created, updated, enabled, disabled, deleted, and
restored transitions and run succeeded and terminal-failed outcomes. Run
progress and retry attempts remain operational. Emit each terminal outcome only
once in the same transaction as terminal-state persistence. Terminal events
carry the originating causation chain; default matching excludes events caused
by that workflow, including indirect failure loops. Preserve enough correlation
to reject cycles across workflows rather than merely checking the immediate
actor. Other producer slices own processing, ingestion, and document-transition
publication. Integrate their catalogs through the shared registry. Trash,
restore, OCR, and thumbnail events receive explicit support or a documented
incompatibility reason instead of falling through an unexamined match arm.

The outbox uses `OutboxEvent.event: Option<DocumentEvent>` and
`envelope: Option<DomainEventEnvelope>`, rejecting rows with neither. Legacy rows
retain the document event, new document transitions carry both, and non-document
transitions carry only the envelope. Check current access before condition
evaluation and delivery.

The document-transition producer proposes tag, correspondent, type, title,
move-out, move-in, purge-requested, purged, trash, and restore events. The trigger
registry must consume these producer-owned catalogs without duplicating
publication. Selecting both a compatibility update event and its specific
transition must not execute the same workflow twice for one mutation. A
correlation ID that spans several mutations cannot identify a single transition.

Immutable `mutation_id` is separate from `event_id` and
`correlation_id`. Producers share it across all specific transitions and the
compatibility Updated event from one mutation. Workflow runs deduplicate by
`workflow_id + mutation_id` as well as preserving legacy source-event dedupe.
Distinct mutations in one correlation chain remain distinct occurrences.

Workflow-run outcomes are non-document subjects even when they reference a
document. Their compatible actions are delivery actions, not document mutations.
The registry owns that distinction, rather than inferring action eligibility
from an optional document ID in a payload.

## Migration allocation

SQLite and PostgreSQL use matching migration filenames. The protected branch at
`c0e840b8` includes the foundation's `20261102160050_domain_event_identity.sql`
and `20261102200300_workflow_lifecycle.sql`. It also includes
`20261102165000_workflow_mutation_dedup.sql` for mutation-level run identity and
`20261102165100_workflow_definition_revision.sql` for captured revisions and
stale-run cancellation.

The staged scheduling and trigger-selection slices use three distinct versions.

| Migration | Scope | Status at `c0e840b8` |
| --- | --- | --- |
| `20261123000000_workflow_clocks.sql` | UTC schedule cursor and occurrence identity | Open in #578 |
| `20261123000100_workflow_additional_triggers.sql` | Ordered additional event selections | Staged locally |
| `20261123000200_workflow_date_offsets.sql` | Document-date offset claims and occurrence identity | Staged locally |

Check main for version collisions before publication. Leave applied migrations
unchanged. The combined upgrade tests cover independent producer PRs landing
in different valid orders.

Document producers publish allowlisted occurrence fields directly in envelope
`data`: title, original filename, MIME type, creation time, classification IDs and
names, tags, and source. Specific transitions add their delta fields alongside
that snapshot. Document matching decodes this immutable occurrence data; it does
not reload current metadata. Current state is used only for authorization and
action eligibility. Missing file paths, OCR, or bytes are deliberate and must not
be reconstructed for condition evaluation. Outgoing move events retain source
scope and incoming events retain destination scope. Purged may carry the legacy
Deleted projection alongside its envelope. Mutation identity groups compatible
Updated and specific deltas from the same write.

Foundation's lineage follow-up defines
`mutation::MutationLineage { correlation_id, causation_id }`,
`with_lineage(Option<MutationLineage>, future)`, and `current_lineage()`. Envelope construction
reads that server-established task-local scope. Workflow actions restore the
captured source correlation and use the source event ID as causation while
running under system actor scope. Persist that source context with the run so a
retry does not depend on an outbox row surviving retention. Spawned work must
establish its own scope; task-local state does not inherit automatically.

Processing event names use `document.processing.{stage}.{outcome}`, with OCR,
embedding, and thumbnail stages. Their catalog fields include run/revision IDs,
attempt, reason code, retry time, and OCR text availability. Use `Timestamp` for
retry instants and `Date` for date-only document fields; do not conflate them.
Managed source lifecycle events use an Owners audience. Active-vault guards in
workflow repositories remain foundation-owned and must be preserved.

Entry events use `document_note.created/updated/deleted` and
`document_comment.created/updated/deleted`. Their subject IDs identify entries;
`document_id` identifies the parent logical document. Data contains entry kind,
transition, and entry ID without body text. Do not add aliases for unpublished
draft event names.

Entry producers are merged in PR #277 and verified in both database adapters.
They publish Vault-audience envelopes with the entry as subject and the parent
in `document_id`. Data contains only `entry_kind`, `transition`, and `entry_id`;
it does not contain a historical document metadata snapshot. Entry dispatch
must not reuse the metadata-event decoder or load current document metadata to
simulate occurrence conditions. Expose only supported entry/envelope conditions
until a producer-owned historical snapshot contract exists. Preserve envelope
attribution and lineage in the durable run context. The next entry increment
follows metadata dispatch PR #291 and includes all six event names, both editors,
and actual-producer replay, move, access-revocation, and recursion regressions.

The pending event-condition extension owns `conditions.event.subject_id` and
`conditions.event.actor`. Actor uses `{ "type": "system" }` or
`{ "type": "user", "user_id": "..." }`. These predicates use the captured
envelope and compose within the same recursive all/any/none branch as document
predicates. Missing snapshots do not satisfy document predicates; missing event
context does not satisfy event predicates. Pre-import definitions reject event
predicates. Transition-specific fields and controls for moves, trash, restore,
and purge remain owned by the document-transition slice. The extension is
prepared locally but awaits runtime verification before publication.

### Envelope and compatibility trigger resolution

Resolve candidates from both the registered envelope event type and the legacy
projection, rather than selecting one representation first. A `document.purged`
envelope may retain a legacy Deleted projection: new purged selections and
existing document.deleted selections must both remain reachable. Candidate
resolution belongs to the shared workflow engine; lifecycle producers must not
append duplicate events or dispatch each representation independently.

Deduplicate candidate names in memory and enforce one run per workflow plus
mutation ID transactionally. A workflow selecting both representations runs once;
two separate workflows selecting different representations may each run once.
Legacy rows without envelopes retain source-outbox-ID deduplication. Never use
correlation ID as the occurrence key. Keep envelope audience, vault, and current
authorization checks shared across all candidates. Preserve each registration's
explicit target: moved-out, trashed, purge-requested, and purged are Event targets;
restored and moved-in are Document targets. An optional document ID does not
change that compatibility classification.
