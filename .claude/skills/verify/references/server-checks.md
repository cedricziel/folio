# Server regression checks

## Verify vault selection

Run `cargo test -p folio-api --test vaults` for discovery, explicit request scope,
role enforcement, revoked membership, scoped redirects, and CSRF protection.
Run the isolated Playwright fixture with `e2e/tests/vaults.spec.ts` for desktop
and mobile navigation, independent upload destinations, viewer controls, and
late search responses after switching vaults. Use the fixture's temporary
database and server, never an existing archive.

The Apple package suite covers immutable scoped services, independent upload
destinations, and external document links. File Provider tests must cover stable
vault-qualified identifiers, scoped downloads and Inbox uploads, read-only
capabilities, membership revocation, and transient failures that retain entries.
Verify that vault discovery 404 remains an error, unloaded catalogs grant no
write capabilities, and UUID-only Spotlight and unscoped provider identifiers
are rejected. Keep current default-vault document Intent coverage separate from
external identifier parsing.
Run both the package suite and native provider tests in [apple-checks.md](apple-checks.md) after these changes.
These tests do not establish system integration in Files or Finder.

## Verify forward authentication

Run `RUSTC_WRAPPER= cargo test -p folio-api --test auth_flow forward::`.
The suite uses temporary SQLite databases and in-process router requests.
It covers identity changes on JSON, fetch, and HTML requests, matching-session
reuse, cookieless session bounds, trusted proxies, shared secrets, repeated
headers, and bearer-request isolation. Do not use a live gateway or user session.

## Verify server shutdown

Run `cargo test -p folio-app shutdown::tests` to check the HTTP drain deadline
and completion or cancellation of background tasks. On Unix, run
`cargo test -p folio-app --test shutdown`. This test starts a child process
with its own SQLite database, storage directory, and loopback ports. It keeps
`/events` connected during SIGTERM, checks clean exit and stream completion,
and receives the final shutdown log through a local OTLP collector.

## Verify ingestion claims

Run `FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test ingest_outcomes --test ingest_health --test ingest_source_scope`
with the isolated Cargo settings below. These tests use temporary SQLite databases
and disposable PostgreSQL containers. They cover atomic outcome/ledger/outbox
rollback, concurrent replay, stale attempts, changed skip decisions, bounded
pagination, managed-source ownership, and source failure/recovery transitions.
The source scope contract also trashes an enabled source during a poll and
checks that new claims, pending completion, and status writes are rejected.
Upgrade cases preserve existing ledger rows and emit no historical outcomes.
Do not treat an unavailable PostgreSQL fixture as a passing parity check.

Folder regressions cover replacement during import, staged retry and restart,
cancellation, concurrent source instances, and symlink-safe recovery.
Observation regressions also cover stable keys across staging restart and mailbox
retry, new keys for replacements and UIDVALIDITY resets, and attachment identity
when allowed content types change. Core identity tests cover length-prefixed
hashing and the Message-ID fallback without publishing raw locators.

Run `cargo test -p folio-infra --test ingest_pipeline --test ingest_folder --test ingest_email`
with a task-specific temporary `CARGO_TARGET_DIR` and bounded `CARGO_BUILD_JOBS`.
The pipeline tests use temporary SQLite databases and in-memory original storage.
The pipeline and importer suites cover recovery before document creation, after
creation, after original storage, and before ledger completion. They also cover
stale-owner fencing, failed storage, and purge during a late object write.
Run `cargo test -p folio-infra --test importer --test workflows --test ingest_pipeline --test repository_contract`
for upload finalization. Gated PUT tests verify that blocked or failed originals
emit no Created event; successful finalization emits one event and workflow run
across retries. The shared contract covers atomic path/event commits, concurrent
finalization, outbox pruning, and already-announced records on both databases.
Outbox context regressions in the repository contract cover immutable metadata
and taxonomy names, transaction rollback, scoped worker events, delete snapshots,
and payload size independent of OCR content. Run `cargo test -p folio-core events::tests`
and `cargo test -p folio-infra --test vault_outbox_upgrade` for wire compatibility.
For grouped workflow conditions, run `cargo test -p folio-core --features openapi --lib`
and `cargo test -p folio-api --test workflows --test openapi`. These cover nested
all/any/none matching, depth and size limits, historical snapshots, legacy JSON,
recursive schema references, and preserving API-created groups through HTML edits.
The core suite also verifies trigger registration, duplicate and unknown selections,
condition catalogs, and action compatibility across document, event, and import targets.
The API fixture uses temporary SQLite state and loopback webhook receivers.
The same core and API suites cover subject and actor conditions, explicit system
attribution, mixed event/document branches, unavailable entry body filters, and
preserving event filters through HTML edits. The workflow suite verifies that
captured actors override ambient dispatcher context without granting private access.
The shared repository contract also verifies concurrent mutation deduplication,
separate workflows, distinct mutations sharing a correlation, invalid captured
identity rejection, and legacy source-event deduplication on both databases.
For precise metadata transitions, `cargo test -p folio-infra --test workflows`
checks actual producer dispatch, delayed snapshots, replay deduplication, and
system-actor lineage after source-outbox deletion. Run
`FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test repository_contract satisfies_the_contract`
for SQLite/PostgreSQL producer-to-run persistence, and the workflow API suite
for trigger selection and web-editor roundtrip.

Run `cargo test -p folio-infra --test workflows` and
`cargo test -p folio-api --test vaults --test live` for immutable event delivery.
These regressions delay metadata, rename taxonomy, move documents before dispatch
and between actions or retries, and verify terminal failures without webhook delivery.
Legacy tests cover missing nondelete context through the attempt limit, deletion
compatibility, and no-document test runs. SSE tests cover origin filtering, live
membership revocation, and exclusion of internal context from the wire. They use
temporary SQLite state and loopback HTTP servers.
They cover competing source claims, owner success/failure/cancellation, missing
originals, and retained trash. Folder tests use temporary inputs; IMAP tests use
loopback fixtures or disposable GreenMail containers. Never point these suites
at an existing watched folder or mailbox.
The IMAP suite also covers reused UIDs after UIDVALIDITY or account changes,
missing UIDVALIDITY, and compatibility with existing message ledger keys.
Deadline tests stall connection setup, commands, partial response streams, and
logout on loopback sockets. They verify socket release, retained ledger state,
and a successful subsequent poll without contacting a real mailbox.
Run the repository contract in [Verify document repository changes](#verify-document-repository-changes) below with `FOLIO_REQUIRE_DOCKER=1` to verify atomic
claim replacement on both SQLite and PostgreSQL. A skipped container test does
not verify PostgreSQL behavior.

## Verify MCP changes

Run `RUSTC_WRAPPER= cargo test -p folio-mcp` for protocol, tool, HTTP, and OAuth
regressions. The skills suite uses in-memory transports and temporary fixture
storage. HTTP tests bind loopback port 0, use test identities, and stop with their
Tokio runtime. They do not start the app or access user archives or credentials.

The skills tests cover discovery, direct lookup, manifest digests and
byte sizes, frontmatter equality, unknown URIs, invalid cursors, standard resource
reads, and document-scope separation. OAuth tests require the resource-specific
metadata URL `/.well-known/oauth-protected-resource/mcp`.
Do not launch a server against existing data.

## Verify global settings authorization

Run `RUSTC_WRAPPER= cargo test -p folio-api --test pipeline_settings`.
The real-router tests use temporary SQLite state and test sessions. They cover
ordinary-user denial, owner/admin access, credential redaction, replacement,
blank preservation, explicit clearing, and cross-vault re-embedding denial.
Provider checks use the local `none` providers and do not contact hosted services.
On macOS, a `system-configuration` NULL-object panic during HTTP-client
initialization can indicate blocked OS networking configuration access. Rerun
with that access while keeping the temporary test state.

## Verify mixed PDF OCR

Run `cargo test -p folio-infra --lib ai::ocr::tests` and
`cargo test -p folio-infra --test mixed_pdf_ocr` with Poppler, Tesseract, and
English Tesseract language data installed. The synthetic PDF has four digital
pages and one scanned page. Verify that only the scanned page reaches the
OCR engine and that its recognized phrase is searchable after storing the text.
The literal search check uses a temporary SQLite database.

## Verify configuration, storage, and provisioning

Run `RUSTC_WRAPPER= cargo test -p folio-infra config::tests --lib` for canonical
environment names, secret files, and ignored obsolete configuration names.
The `storage_contract` integration suite covers canonical object keys and prefix
deletion. API `routes` tests cover canonical previews and ignore old preview keys.
The `vault_isolation` suite checks that provisioning does not claim ownerless
archives and that retained SQL migrations still preserve existing rows.

Before a PR, also run `RUSTC_WRAPPER= make format`, `RUSTC_WRAPPER= make lint`, and
`RUSTC_WRAPPER= make test`. An empty wrapper bypasses a local sccache service when
the sandbox cannot access it.
Coordinate one Cargo build across delegated checks. Concurrent package-specific
commands share the build lock and can compile different dependency feature sets.
Use the workspace suite when several crates change.

## Verify document thumbnails

Run `RUSTC_WRAPPER= cargo test -p folio-api --test thumbnails` with isolated build
storage. The real-router tests cover generated JPEG bytes, unavailable thumbnails,
store failures, bearer authentication, read scope, and vault/account isolation.
The server smoke test in `.claude/skills/verify/scripts/smoke.sh` also retrieves
the processed fixture's thumbnail and checks its JPEG bytes and private no-store
headers. Use that skill's disposable instance wrapper, never a user archive.

`LiveFolioServiceTests` covers bearer and vault headers, token refresh, unavailable
thumbnails, authentication errors, and JPEG byte/pixel limits. Native
`FolioProviderTests/ThumbnailTests` covers independent per-item results, folder
and invalid-identifier handling, empty batches, and cancellation that stops the
network transfer and completes the batch once. Run the isolated package and
macOS provider commands in [apple-checks.md](apple-checks.md); these tests do not register a provider domain.

`ThumbnailCacheTests` covers revision invalidation, workspace isolation, retry,
cancellation, and eviction. `WorkspaceTests` verifies cache replacement on vault
selection. Run them with the isolated package command in [apple-checks.md](apple-checks.md).

`apple/ThumbnailTests/project.yml` builds the production document row with a mock
service. Generate its project outside the checkout and run the `Fixture` scheme
on macOS and a fresh iPhone simulator, using the isolation and 180-second test
timeouts described under Apple inbox navigation in [apple-checks.md](apple-checks.md). The test captures placeholder
and preview screenshots, then opens rows with PDF/image previews, missing
thumbnails, and authentication failures. Inspect the retained screenshots for
preview visibility and uncropped content. It uses no account, keychain, server,
or File Provider domain. Remove its simulator and temporary build on every exit.
If the macOS UI runner exits before connecting to XCTest, report that failure
separately. The standalone fixture can be checked with native UI automation:
launch with `--without-thumbnails` for the placeholder baseline, relaunch without
it, inspect the previews, and click each row to verify its `Opened:` label. This
verifies row interaction without claiming that the XCTest runner passed.
The Apple workflow also runs the iPhone fixture in the `Thumbnail UI` job and
retains its result bundle, including screenshot attachments, for seven days.

## Verify saved-view contracts

Run `cargo test -p folio-core --all-features` with the isolated Cargo settings
above. Saved-view tests cover query versions, unknown fields, bounded paging,
date ranges, owner-only private visibility, shared edit permissions after a role
change, soft deletion, and rejection of client-supplied ownership or actors.
These tests exercise models only and do not launch an app or access user data.

Run `cargo test -p folio-infra --test saved_views` for saved-view persistence.
It requires Docker and uses temporary SQLite files and disposable PostgreSQL
containers. The shared contract covers owner isolation, shared edit permissions,
soft deletion, name reuse, actor attribution, and trusted event lineage. It also
checks that private transitions remain owner-only, event payloads omit queries,
unchanged and concurrent duplicate updates emit no extra event, and a forced
outbox failure rolls back creates, updates, and deletes. Never use an existing
archive for these checks.

Run `cargo test -p folio-api --test saved_views --test openapi` for the JSON API.
Temporary SQLite fixtures cover private visibility, viewer sharing restrictions,
cross-vault filter rejection, rename precision, audit identity, and deletion.
PATCH must reject explicit null and client-supplied ownership. Regenerate the
OpenAPI contract with `make openapi` when the route schema changes.

## Verify document repository changes

The shared repository contract covers atomic OCR completion and its outbox event,
including empty output, concurrent repeats, unchanged results, rejected writes, and
rollback when the outbox rejects insertion. The worker suite also covers retries
after an empty OCR completion without repeating OCR or its completion event.

Run `RUSTC_WRAPPER= FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test repository_contract`
with Docker available. The shared contract uses disposable SQLite databases and
PostgreSQL containers; it covers bounded pagination, stable ordering, cursor
binding, and batched tags with duplicate IDs and vault isolation. Containers are
removed after the tests. Do not point these tests at an existing archive.

The semantic search contract uses fixed embeddings and each backend's real search
store. It checks filtered recall beyond the global candidate cutoff, inclusive
creation start and exclusive end, deterministic score ties, trash exclusion, and
vault isolation. Run the SQLite and both PostgreSQL configurations.

For paged HTTP browsing, run `cargo test -p folio-api --test vaults browse`.
The route tests cover bounded continuations, filter/tag enrichment, malformed
queries, legacy list compatibility, invisible vaults, and membership revocation
between pages. Search coverage verifies title/content matches outside the first
unfiltered page, AND semantics for tags, and rejection of cursors after text
changes. The shared route harness owns a temporary SQLite directory and
removes it when dropped. Regenerate OpenAPI with `make openapi`; the Apple schema
input is a symlink to that file, so run the isolated Swift package suite in [apple-checks.md](apple-checks.md)
after schema changes.

When temporary build storage is constrained, run Rust and Apple verification
sequentially. Use `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` consistently
for the Rust run and a task-specific `CARGO_TARGET_DIR`; remove that target after
verification before starting the Apple build. These settings omit debug symbols,
not tests.
Before reusing a Rust target directory from another checkout, run `cargo clean -p PACKAGE`
for each workspace package, replacing `PACKAGE` with its name. Retain the dependency cache, but rebuild workspace
packages so stale artifacts from the other checkout cannot satisfy a check.

## Verify processing trigger definitions

Run `cargo test -p folio-core --test processing_trigger_catalog --test processing_events`
for producer event names, retry versus terminal failure choices, and bounded typed
condition fields. Catalog tests do not establish registry dispatch or builder UI;
verify those through the workflow suites above when registering the catalog.

## Verify worker leases

Run `cargo test -p folio-infra --test processing_job_context` for enqueue-time vault
and trusted lineage retention through retry and vault move in both databases.
Worker `processing_claim` tests cover restored source lineage, system attribution,
and rejection before work starts in a different vault. This preflight check does
not establish transactional result fencing for moves during provider execution.


Run `RUSTC_WRAPPER= FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test queue_leases`
against disposable SQLite and PostgreSQL fixtures. The shared contract covers
renewal of long-running work, expired-owner recovery, stale renewal and completion,
stale retry and failure, and recovery of processing rows created before leases.
Run `RUSTC_WRAPPER= cargo test -p folio-infra --lib queue::worker::tests`
for heartbeat renewal, rejection before expired work starts, cancellation on
lease loss or renewal timeout, and progress while renewal waits for a connection.
The worker suite rejects missing vectors and changes to the embedding model or
dimensions before storing chunks. A consistent retry stores every chunk.
Run `cargo test -p folio-core --test embedding_batch` for missing or surplus
vectors, malformed dimensions, and nonfinite provider output.
Run `cargo test -p folio-core --test processing_attempts` for retry state, stale
attempt rejection, terminal immutability, persisted-state restoration, and attempt
counter exhaustion. These model tests do not verify transactional lease fencing.
Run the repository contract as well after changes to queue method signatures.
These tests use temporary or in-memory data.

## Verify document trash

Run `cargo test -p folio-api --test trash --test vaults --test live` for default
soft deletion, retained originals, restore, explicit purge, permissions, and
vault-scoped events. Run `cargo test -p folio-infra --lib queue::worker::tests`
for cleanup retries, indefinite retention, and processing races. The repository
contract above also verifies transactional purge enqueueing on both databases.

Build `folio-app`, then run `cd e2e && npx playwright test trash.spec.ts documents.spec.ts shortcuts.spec.ts`.
The setup script starts an instance in a temporary directory with its own port,
database, storage, and test configuration; teardown removes it. Do not set
`E2E_BASE_URL` to an existing user instance. Verify that Move to Trash preserves
the document, Restore returns it, and Delete forever requires confirmation and
eventually removes its pending row. No automatic retention expiry is supported.
Include `vaults.spec.ts` and the API `vaults` suite when library selection changes:
Trash links, forms, event refreshes, and redirects must retain the selected vault,
and viewer/contributor restrictions must also apply to trashed documents.

For Linux visual checks on macOS, a disposable pinned Playwright browser-server
container can connect to the temporary macOS app through
`PW_TEST_CONNECT_EXPOSE_NETWORK=<loopback>`. Verify the remote browser reports
Linux, then use a temporary harness that removes only the host-platform skip in
`visual.spec.ts`. Keep the same tour fixtures, viewport, masks, and screenshot
paths. Recheck without updating snapshots, and remove the container and harness.

## Verify bounded MCP originals

Run `RUSTC_WRAPPER= cargo test -p folio-core ports::storage::tests --lib`
for oversized metadata rejected before reading, missing or understated sizes,
and successful collection at the exact byte limit. The instrumented stream tests
check that collection stops before consuming the rest of an oversized original.
Run the MCP suite above for the small-file resource response and the oversized
file error that points clients to the text resource.

## Verify integration access

Run `RUSTC_WRAPPER= cargo test -p folio-infra --test personal_tokens` for hashed
credential storage, owner isolation, audience, expiry, and revocation. Set
`FOLIO_REQUIRE_DOCKER=1` to require the disposable PostgreSQL contract instead
of allowing its explicit infrastructure skip.
Run `RUSTC_WRAPPER= cargo test -p folio-api --test personal_tokens` for
session-only management, explicit bearer precedence, and optional-auth scopes.
Run `RUSTC_WRAPPER= cargo test -p folio-api --test integrations` for navigation
and configured connection URLs. Run `RUSTC_WRAPPER= UPDATE_OPENAPI=1 cargo test -p folio-api --test openapi`
to regenerate and validate the API contract.
Run `RUSTC_WRAPPER= cargo test -p folio-mcp --test oauth` for token revocation,
live vault roles, and legacy session ownership. Session tests must negotiate
`2025-06-18`; newer MCP protocol versions can use stateless transport and miss
replay regressions.

Build `folio-app`, then run `npx playwright test integrations.spec.ts` from
`e2e/` against its disposable fixture. It covers desktop and mobile shortcuts,
creation, clipboard copy, reload redaction, revocation, and rejected revoked
credentials. Never supply an existing archive as `E2E_BASE_URL`.
On macOS, sandbox restrictions can block system networking initialization,
loopback listeners, or the Docker socket. Rerun with those capabilities while
retaining temporary data and test credentials. Record a skipped backend as
unverified.

## Verify domain ownership and event contracts

Run `RUSTC_WRAPPER= cargo test -p folio-core --lib` for isolated mutation actors,
versioned envelope validation, and legacy deletion-snapshot decoding.
Run `RUSTC_WRAPPER= FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test repository_contract --test domain_events --test workflows --test vault_outbox_upgrade`
against disposable SQLite databases and PostgreSQL containers. These suites cover
explicit vault ownership, mismatched workflow/ingestion ownership rejection,
transaction rollback, concurrent event identity deduplication, stable delivery
retries, retained legacy payloads, and workflow suppression for private or
wrong-vault envelopes. Use task-owned build storage and the isolation settings
above. PostgreSQL skips do not establish database parity.

## Verify mutation actors

Run `RUSTC_WRAPPER= cargo test -p folio-core --lib` and
`RUSTC_WRAPPER= cargo test -p folio-api --test mutation_actor`, then the MCP suite.
These regressions cover server-derived session, OAuth, and personal-token actors,
forged input, optional authentication, concurrent requests, cancellation, and
system task isolation. HTTP/MCP fixtures use temporary databases, test identities,
and local transports; they do not access user sessions or credentials.


The core suite also covers trusted event lineage: nested clearing and restoration,
cancellation, concurrent scopes, detached tasks, actor independence, and envelope
validation. Workflow actions must scope server-owned correlation and causation
before repository mutations; request bodies cannot establish this context.

## Verify workflow configuration lifecycle

Run `RUSTC_WRAPPER= FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test workflow_lifecycle --test workflows --test repository_contract`
with disposable SQLite and PostgreSQL fixtures. The lifecycle contract covers
server actors, no-op timestamps, retained configuration and run history, deletion
rollback, terminal-state fencing, inactive queue admission, and legacy upgrades.
Restore must preserve failed pending runs rather than replay their queued jobs.
Run `RUSTC_WRAPPER= cargo test -p folio-api --test workflows --test vaults` for
management controls, deleted-workflow history, secret masking, and vault access.
Regenerate OpenAPI after response or route changes. Never verify workflow delivery
against an existing webhook receiver or user archive.

## Verify managed source lifecycle

Run `RUSTC_WRAPPER= FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test source_lifecycle`
with disposable SQLite and PostgreSQL fixtures. The shared contract covers actor
metadata, retained import ledger and status, disabled scheduling after deletion,
claim fencing, active-name reuse, restore conflicts, no-op suppression, and atomic
source/outbox rollback. Configured sources remain outside managed lifecycle storage.
Run `RUSTC_WRAPPER= cargo test -p folio-api --test sources --test vaults` for the
existing management page, deleted-source listing, restore controls, masked credentials,
and vault authorization. Regenerate OpenAPI and run the isolated Apple package suite in [apple-checks.md](apple-checks.md) when changing source response fields. Never poll an existing folder or mailbox
while verifying source management.

## Verify classification lifecycle

Run `RUSTC_WRAPPER= FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test taxonomy_lifecycle --test repository_contract`
for SQLite/PostgreSQL soft deletion, stable identity, retained document references,
name reuse, restore conflicts, actor metadata, and transactional lifecycle events.
Run `cargo test -p folio-api --test routes --test web --test openapi` for scoped
JSON and management routes, then regenerate OpenAPI and run the isolated Apple
package suite in [apple-checks.md](apple-checks.md). Native tests cover scoped taxonomy requests and restore
conflicts that preserve the displayed trash entry.

Build `folio-app`, then run `npx playwright test classification-lifecycle.spec.ts`
from `e2e/` against its disposable setup instance. The suite covers all three
classification types, retained labels, restore conflicts, vault isolation, and
viewer restrictions. Inspect its active/trash screenshot attachments. Use the
isolated Classifications fixture to check large text, loading errors, and Retry.
The fixture shows the screen in a window, so check macOS sheet sizing by opening
Classifications from the WorkspaceView sidebar; package tests do not verify layout.

## Verify retained vaults and owner grants

Run `RUSTC_WRAPPER= FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test vault_lifecycle` against disposable SQLite and PostgreSQL fixtures. It covers scoped actor/lineage metadata, atomic initial ownership, no-op and rollback suppression, protected defaults, retained references, owner-only recovery, permanent audit history after outbox pruning, lifecycle history retained after more than 100 newer document audit events, and cancellation/fencing of pending workflow runs.
Run `RUSTC_WRAPPER= cargo test -p folio-api --test vault_lifecycle --test vaults` for owner, viewer, and unrelated-admin boundaries, forged lifecycle fields, deleted-vault scoping, and management controls. Existing document-focused drains must acknowledge every event while returning only document projections; dedicated lifecycle contracts inspect all envelopes.
Run the existing repository, workflow, vault-isolation, and ingestion contracts after changes to parent-vault activity checks. A retained vault must not start workflow runs or source polls while deleted; restoring it must not reopen terminal runs.
Build the disposable E2E fixture and run `npx playwright test vault_lifecycle.spec.ts vaults.spec.ts` from `e2e/` for desktop/mobile creation, deletion, restoration, and existing vault isolation. Do not point these tests at an existing archive. The Apple catalog and destination picker do not implement lifecycle management; native metadata, owner controls, recovery, and isolated native regressions remain required follow-up work.

## Verify document notes and comments

Run the [document entry checks](document-entries.md).

## Verify document metadata transitions

For date edits, metadata events, moves, and their API/MCP entry points, run the
[document metadata checks](document-metadata.md).

## Verify saved-view web controls

Run `cargo test -p folio-api --test saved_views` for form CSRF protection,
unavailable references, and exact date-boundary preservation through the editor.
Build `folio-app`, then run
`npx playwright test vaults.spec.ts --grep 'saved views restore|submillisecond'`
from `e2e/`. Set `FOLIO_BIN` to the built binary and leave `E2E_BASE_URL` unset
so setup owns the disposable instance. Desktop and mobile checks save, reopen,
rename, share, and delete views. Inspect screenshots and confirm that a single
grid card keeps a bounded width. The precision regression renames a view without
rounding its stored submillisecond boundaries to the date input's display value.

Shared navigation changes also require the Linux `visual.spec.ts` suite.
Inspect its image diffs and update affected baselines before merging.

## Verify typed metadata fields and options

Run `cargo test -p folio-core --lib --features openapi` for complete long-text event
snapshots, exact serialized-byte limits, and audience/target validation on decode.

Run `RUSTC_WRAPPER= FOLIO_REQUIRE_DOCKER=1 cargo test -p folio-infra --test metadata_fields --test metadata_options`
with task-owned build storage, at most two jobs, incremental compilation disabled,
and zero debug symbols when capacity is constrained. The shared contract uses a
temporary SQLite database and a disposable PostgreSQL container. It covers scoped
field/option reads, server-derived attribution, canonical name uniqueness, concurrent
claims, pagination, retirement, restore conflicts, no-op suppression, and rollback
when outbox publication fails, option identity, and select-field integrity. Events are
vault-visible non-document events. A Docker access failure requires rerunning the
PostgreSQL case with socket access; a skipped backend is not verified parity.

## Verify SQLite trash contention

Run `cargo test -p folio-infra --test trash_lock` with an isolated target.
The temporary database fixture holds a competing writer, verifies trash waits,
and checks that the committed trash snapshot includes the writer's change.

## Verify permanent deletion events

For permanent deletion, `cargo test -p folio-infra --lib permanent_deletion`
checks failed object removal, repair of exhausted jobs, preserved causal lineage,
and system-actor completion. The document transition suite checks distinct purge
request/completion snapshots, repeat suppression, legacy job payloads, and both
databases' persisted request lineage. Regenerate OpenAPI after projection changes.

