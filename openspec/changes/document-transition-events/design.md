# Document transition event contract

Depends on domain-lifecycle envelope and expand-workflow-builder registry. Integrate producers only after those shared contracts land.

## Events

Every event uses the foundation identity, actor, vault, subject, version, correlation, and causation fields. Immutable context captures the occurrence state without OCR text or document bytes.

| Event type | Delta |
| --- | --- |
| document.tags_changed | Sorted unique before_ids, after_ids, added_ids, removed_ids |
| document.correspondent_changed | Nullable before_id and after_id |
| document.document_type_changed | Nullable before_id and after_id |
| document.title_changed | before and after strings |
| document.moved_out | Source-vault occurrence snapshot |
| document.moved_in | Destination-vault occurrence snapshot |
| document.purge_requested | Retained trash snapshot at first request |
| document.purged | Last retained snapshot after committed removal |
| document.trashed | Snapshot at trash transition |
| document.restored | Snapshot at restoration |

Tags have one delta event per committed mutation. The builder offers added, removed, or replaced conditions from the delta, including exact classification IDs. Assignment, replacement, and clearing use nullable ID comparisons. These choices describe outcomes rather than which HTTP operation was called.

## Publication and compatibility

Derive deltas within the mutation transaction from committed before/after values. Reject invalid references before publication. Rollbacks, repeated requests, duplicate tag IDs, and reordered equivalent tag sets emit no events. Append specific transitions and the legacy Updated event atomically. A workflow selects one trigger, so a generic update subscription does not also subscribe to its specific transition events. If the shared engine supports multiple triggers, deduplicate a workflow by the shared mutation identity.

Each event keeps its own retry-stable identity. Conditions use immutable occurrence snapshots. Actions resolve current state and re-check current vault authorization. Deleted subjects permit only actions supported by the registry; failures remain readable in both clients.

## Moves and purge

A move publishes two scoped events in the move transaction. The source record contains only source-vault state, and the destination record contains only destination-vault state after classification remapping. Neither record includes the other vault's private IDs or classifications. Source-triggered actions must not follow the document into the destination vault.

Purge requested fires once after successful enqueueing, distinct from trashed. Purged fires only after committed row removal. Delivery and cleanup retries retain the original identity and cannot duplicate workflow execution. Existing deleted-wire compatibility remains available through the foundation mapping.

## Clients

Extend the shared registry and builder, not a separate dashboard. Use existing design tokens. Provide plain event labels, nullable assignment conditions, tag-added/removed conditions, and visible unsupported-action or unavailable-subject errors. Regenerate OpenAPI and the Swift client after registry schema changes.

## Integration seams

Foundation confirmed public SQLite and PostgreSQL `insert_domain_event(conn, &DomainEventEnvelope)` on the caller's transaction. `data` is immutable JSON; audience is explicit. Document adapter integration waits for foundation attribution edits to land.

Correlation can span several mutations. Multi-trigger workflow deduplication therefore requires a distinct mutation identity shared by the specific deltas and compatibility update for one transaction. Foundation accepted envelope `mutation_id` and `with_mutation(mutation_id)`. One mutation ID is shared by all events from that transaction; correlation alone is not a valid substitute.

The common delta helper compares document metadata and tag ID sets. The revision slice adds nullable `document_date` to `UpdateDocument`, preserving omitted versus explicit clear; its date delta will compose at this same boundary. Review and revision operations remain owned by that slice.

Existing ingress converges on `DocumentRepository::update`, `set_tags`, `add_tag`, and `remove_tag`. HTML and JSON routes call these methods directly; MCP calls the library service, which calls the same methods. Workflow actions use the workflow-origin scoped repository. Cover behavior primarily in the shared repository contract, with route/MCP tests for context propagation. The initial scan found no separate bulk document-write implementation.

## Migration reservation

If integration requires an additional schema change, this slice reserves valid timestamp versions within `20261102172000`–`20261102172999`, with identical versions for SQLite and PostgreSQL. Merged personal tokens own `20261102160000`. Foundation owns `20261102160050_domain_event_identity` and `20261102160100` through `20261102160400`. Check main for collisions before adding migrations. Never renumber merged or applied migrations. Test fresh databases and upgrades with dependency PRs landing in different orders. This reservation does not remove the prerequisite merge gate or require an empty migration.

Downstream integration must preserve scoped personal tokens from PR #251 and immutable delayed-event snapshots from PR #252. Never rename the merged personal-token migration.
