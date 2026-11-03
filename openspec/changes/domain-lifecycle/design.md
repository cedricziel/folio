# Domain lifecycle contract

This is the integration contract for dependent domain slices. Implementation is in progress.

## Fields

Mutable domain entities expose `created_at`, `updated_at`, `deleted_at`, `created_by`, and `updated_by` as top-level fields. Times use UTC RFC 3339. Actor IDs are nullable strings using the existing `UserId`. `deleted_at` is nullable. Every vault-owned response has a required UUID `vault_id`. Write request bodies do not accept lifecycle fields or ownership overrides.

Creation sets both times to the same server time and both actors to the authenticated actor. Mutation preserves creation metadata and replaces update metadata. System jobs use null, rather than impersonating an uploader or workflow author. Legacy unknown actors remain null. `uploaded_by` retains its contributor-authorization meaning and is not replaced by audit metadata.

## Deletion and references

Default reads and selectors omit deleted objects. Deletion retains identity and references. New assignments reject deleted or cross-vault references. Existing document classifications remain readable and are labeled deleted. Name uniqueness applies to live rows within a vault. Restoration fails with a conflict when a live object has reused the name. Restoration never silently renames or merges objects.

Deleted workflows and ingestion sources stop scheduling new work. Existing immutable event payloads, action snapshots, and completed history are not rewritten. Restore does not replay historical events or restart completed ingestion.

Document trash and irreversible purge retain their existing semantics. A document move preserves creation metadata, sets update attribution to the mover, remaps live classifications by name, and writes audit events to both vaults. The caller must have the required access to both vaults. Other entity ownership cannot be changed through ordinary updates.

## Actor boundary

Only server authentication establishes a user actor for HTTP and MCP mutations. Background tasks establish system context. Actor context must not leak across concurrent requests or detached tasks. Actor values supplied in JSON are never trusted. Authorization remains explicit and vault-scoped independently of attribution.

## Exceptions

Audit events and outbox payloads are immutable facts with their original timestamp and actor. They do not receive artificial update or deletion fields. Job leases, retry status, ingestion claims, credentials, join rows, derived embeddings, and search indexes are operational records rather than user-managed domain entities. Their existing expiry, revocation, retention, and parent lifecycle rules continue to apply. Mutable workflow-run execution state is operational; its captured actions and event payload remain immutable.

## Client behavior

JSON additions are read-only metadata, documented in OpenAPI and regenerated into the Swift client. Web and Apple interfaces display creation/update attribution and deletion state, offer permitted trash/restore actions, and show restore conflicts without losing the retained object. Shared design tokens supply all styling values.

## Persisted object inventory

| Tables | Lifecycle treatment |
| --- | --- |
| documents, tags, correspondents, document_types, workflows, ingestion_sources | Mutable vault domain; full lifecycle fields and explicit vault ownership |
| vaults | Mutable ownership root; full lifecycle fields without a redundant vault_id |
| users, auth_groups | Global security principals; preserve account/group revocation rules and audit identities; account deactivation is a separate capability |
| vault_members, auth_group_members | Authorization grants; revocation removes active membership, audit events retain grant history |
| audit_events | Immutable history; original at/actor values, no synthetic lifecycle fields |
| workflow_runs | Operational execution state with immutable event/action snapshots; ownership projected from workflow |
| ingest_ledger | Idempotency claims and completed outcomes; explicit vault ownership, no soft-delete that permits duplicate import |
| outbox_events | Immutable event payload plus mutable dispatch progress |
| settings, oauth_clients | Global operational configuration; retain existing management and revocation semantics |
| sessions, oauth_codes, oauth_access_tokens, oauth_refresh_tokens, oidc_logins, user_totp, user_recovery_codes, user_passkeys | Security credentials; existing expiry, revocation, and hard-deletion semantics |
| jobs, ingestion_status, pipeline_initialization | Operational progress, leases, and initialization state |
| document_tags | Parent-owned associations; document mutation records changes |
| document_chunks, document_embeddings | Derived document data, rebuilt or purged with the parent |

Configuration-defined ingestion sources are not persisted domain entities. Their existing optional vault binding remains valid; managed source responses always contain the persisted vault.

## Domain event envelope

Extend the existing durable outbox with `DomainEventEnvelope` version 1. Keep the monotonic outbox row ID as the delivery/SSE cursor. Each new committed transition receives a UUID event identity that retries preserve.

The envelope contains `schema_version`, `event_id`, `mutation_id`, `event_type`, `occurred_at`, `vault_id`, `subject` with `type` and `id`, nullable `actor_id`, `correlation_id`, nullable `causation_id`, nullable `document_id`, explicit `audience`, and immutable allowlisted `data`. `audience` is `Vault`, `User { user_id }`, or `Owners`. Producers define typed payload fields; the shared envelope validates shape and size. Actor IDs use the same server-derived context as lifecycle writes. A non-document subject has no implied document target. Consumers reject unsupported actions explicitly.

Use stable dotted names such as `document.created`, `document.updated`, `document.trashed`, `document.restored`, `tag.created`, and `tag.updated`. Preserve existing document wire event names through compatibility mapping. The workflow capability supplies user-facing trigger labels and condition schemas.

Repositories append events in the same transaction as the state change. Rollbacks and no-ops publish nothing. Retrying delivery preserves event identity and the existing workflow/source-event uniqueness constraint. Events from one mutation share a mutation UUID. Correlation may span multiple mutations and must not suppress later legitimate changes. Workflow-caused mutations carry correlation and causation identifiers for recursion protection. Moves create scoped records for each affected vault without exposing destination-private fields to source-only readers. Deleted subjects remain understandable from their immutable event snapshot. Private user-owned subjects require an explicit event audience and consumer authorization; arbitrary private content must not enter a vault-wide payload.


`OutboxEvent.event` becomes optional and `OutboxEvent.envelope` is optional for legacy rows. New document events retain the compatible document projection alongside the envelope. Non-document events have only the envelope. Rows with neither representation are invalid. A new nullable unique `outbox_events.event_id` column provides atomic append deduplication on both databases in `20261102160050_domain_event_identity.sql`.

`insert_domain_event(&mut Connection, &DomainEventEnvelope)` is public in each database adapter and requires the caller's transaction. Event identity survives delivery retries; an attempt to reuse an identity with different content fails. The dispatcher continues using the existing outbox and monotonic cursor.

Vault membership granted, role changed, and revoked events use owner-only audience, as do vault lifecycle events. Consumers recheck current membership before reading payloads or executing workflows. A deleted vault has no active workflows; its retained audit events support authorized historical inspection, not post-deletion execution.

## Retained vaults and membership facts

Vaults retain their existing non-unique names. `created_by` and the new nullable `updated_by` use trusted mutation actors; legacy update actors remain unknown. `deleted_at` hides a vault without deleting documents, sources, workflows, grants, or completed history. Current direct or group owners may discover deleted metadata and restore it. Instance administrators have no implicit vault role. Ordinary access, discovery, and default resolution exclude deleted vaults.

Creation with an initial owner writes the vault and direct owner grant in one transaction. The current actor is independent of the initial owner identity. The last direct user owner remains protected. Identical grants/renames and repeated revocations/deletions/restorations emit no facts and preserve metadata. Referenced account defaults and the active instance default require an explicit replacement before deletion; stale defaults without current access do not block an owner indefinitely.

Vault facts use `vault.created`, `vault.updated`, `vault.deleted`, and `vault.restored`, with subject type `vault` and the vault UUID. Grant facts use `vault_membership.granted`, `vault_membership.role_changed`, and `vault_membership.revoked`, with subject UUIDv5 derived from the vault namespace and `user:<id>` or `group:<id>`. Their audience is always `Owners`, and `document_id` is null. Initial vault/owner facts share a mutation UUID. Payloads contain allowlisted lifecycle metadata or principal IDs and role transitions. Matching compact audit rows commit in the same transaction and survive outbox retention.

Deletion terminally fails pending workflow execution while preserving captured actions, event payloads, completed actions, and existing terminal history. Scheduling and each action recheck active parent-vault state. Late execution cannot reopen a terminal row. Source scheduling and ingestion-bound sinks also recheck the parent vault; restoration does not clear ingestion history or replay completed work. An external request already issued before deletion cannot be recalled.

The web management surface exposes owner controls, account-default selection, retained-vault recovery, and owner-visible history using existing shared styles. The Apple catalog and destination picker remain existing read/navigation surfaces. Native lifecycle metadata, management, recovery, and regressions are outstanding work in this change, not an exception to the cross-client requirement.

An owner with no active vault can still reach account-level vault management and explicitly restore retained vaults, including after a shared default is revoked. These recovery pages establish no request vault and start no scoped event stream. Ordinary document routes remain inaccessible. Vault discovery returns a null `default_vault_id` when its active list is empty; ordinary populated catalogs retain their existing UUID value. The runner also checks persisted terminal state before every action, so delete-and-restore cannot revive an already-running stale snapshot.
